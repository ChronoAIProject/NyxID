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
        bot_owner_id: None,
        platform: if transport == "gateway" {
            "telegram"
        } else {
            "lark"
        }
        .into(),
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
        gateway_groups: None,
        owner_sender_ids: Vec::new(),
        link_code_hash: None,
        link_code_expires_at: None,
        source_conversation_id: None,
        agent_id: None,
        private_chats: None,
        delivery_status: None,
        delivery_error: None,
        delivery_failed_at: None,
        delivery_seen_at: None,
        delivery_checked_at: None,
        delivery_notified_at: None,
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
    let stranger =
        body_text(respond(&state, &agent_key, &event("hi", "42", "evt-1"), "evt_1").await).await;
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
        assert!(instructions.contains("channel bot. Replies are delivered"));
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
    // A message while this chat's turn is still running is queued for the
    // next turn (answered later as an update), not bounced.
    state
        .db
        .collection::<mongodb::bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &conversation.id},
            doc! {"$set": {"active_turn": {
                "turn_id": "busy", "origin": "channel",
                "started_at": mongodb::bson::DateTime::now(), "stop_requested": false,
            }}},
        )
        .await
        .unwrap();
    let busy = body_text(
        respond(
            &state,
            &agent_key,
            &event("And one more thing", "7", "evt-4"),
            "evt_4",
        )
        .await,
    )
    .await;
    assert!(busy.contains("will answer this right after"), "{busy}");
    let queued = crate::services::assistant_nyxagent::get(&state.db, OWNER, &conversation.id)
        .await
        .unwrap();
    assert!(
        queued
            .pending_events
            .iter()
            .any(|event| event.kind == "message" && event.text.contains("And one more thing"))
    );
    assert_eq!(calls.lock().await.len(), 1);
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

/// The gateway authenticates a channel's creator with NyxID
/// `GET /api/v1/users/me`, classifies the subject with
/// `GET /api/v1/orgs/{subject}/authorization` (404 means a person), and
/// requires `profile.metadata.owner.subject` to match. Replays that check with
/// the exact bearers `connect` uses.
#[tokio::test]
async fn gateway_creator_check_accepts_the_owner_bearer_nyxid_sends() {
    let (state, _, server) = setup("nyxbot_creator_check").await;
    // `/users/me` resolves the platform role that startup seeds.
    crate::services::role_service::seed_system_roles(&state.db)
        .await
        .unwrap();
    let (_, private) = crate::routes::build_router_with_state(state.clone());
    let app = private.with_state(state.clone());
    let call = |method: &str, path: String, bearer: String| {
        let app = app.clone();
        let request = axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {bearer}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(if method == "GET" {
                ""
            } else {
                "{}"
            }))
            .unwrap();
        async move {
            let response = tower::ServiceExt::oneshot(app, request).await.unwrap();
            let status = response.status();
            (status, body_text(response).await)
        }
    };
    let creator = creator_bearer(&state, OWNER).unwrap();
    let (status, body) = call("GET", "/api/v1/users/me".into(), creator.to_string()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let me: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(me["id"], OWNER);
    let (status, body) = call(
        "GET",
        format!("/api/v1/orgs/{OWNER}/authorization"),
        creator.to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    // The creator bearer reads; it cannot change the account.
    let (status, _) = call("PUT", "/api/v1/users/me".into(), creator.to_string()).await;
    assert!(status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED);
    // The channel's agent key (the provider bearer) is refused as a creator,
    // which is why channel management uses the creator bearer.
    let agent = create_gateway_agent_key(&state, OWNER, "Helper bot")
        .await
        .unwrap();
    let (status, _) = call("GET", "/api/v1/users/me".into(), agent.full_key.clone()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    server.abort();
}

/// Chat apps cannot show NyxID's confirmation cards: the verified owner's
/// plain "yes" decides the pending one and the agent retries.
#[tokio::test]
async fn owners_confirm_actions_by_replying_yes_in_the_chat_app() {
    use crate::services::assistant_acknowledgement_service as acks;
    let (state, calls, server) = setup("nyxbot_chat_confirm").await;
    let (row, agent_key) = channel(&state, "gateway").await;
    put_binding(
        State(state.clone()),
        Path("bnd_confirm".into()),
        bearer(&agent_key),
        Json(binding_body(&agent_key, OWNER)),
    )
    .await;
    put_conversation(
        State(state.clone()),
        Path(("bnd_confirm".into(), PARTITION.into())),
        bearer(&agent_key),
    )
    .await;
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"owner_sender_ids": ["7"]}},
        )
        .await
        .unwrap();
    let first = body_text(
        respond(
            &state,
            &agent_key,
            &event("Delete agent key ci-bot", "7", "evt-1"),
            "evt_1",
        )
        .await,
    )
    .await;
    assert!(first.contains("Here is your answer"), "{first}");
    let conversation: AssistantConversation = state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"user_id": OWNER, "channel.nyxbot_channel_id": &row.id})
        .await
        .unwrap()
        .unwrap();
    // The destructive tool asked for confirmation during that turn.
    let chat = acks::for_key(&state.db, OWNER, Some(&conversation.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let (card, _) = acks::request_tracked(
        &state.db,
        &chat,
        acks::Request {
            kind: "action",
            service: None,
            tool: Some("nyxid__delete_agent_key"),
            arguments: Some(&json!({"key_id": "ci-bot"})),
            summary: "Delete agent key 'ci-bot'",
            platform: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(card.status, "pending");
    // An ordinary message is not an answer.
    assert_eq!(acks::parse_reply("what does that do?"), None);
    assert_eq!(
        acks::parse_reply("Yes 4821."),
        Some((true, Some("4821".into())))
    );
    let second =
        body_text(respond(&state, &agent_key, &event("Yes!", "7", "evt-2"), "evt_2").await).await;
    assert!(second.contains("Here is your answer"), "{second}");
    let decided = acks::history(&state.db, OWNER, &conversation.id)
        .await
        .unwrap()
        .into_iter()
        .find(|ack| ack.id == card.id)
        .unwrap();
    assert_eq!(decided.status, "allowed");
    assert_eq!(decided.decided_by.as_deref(), Some("user"));
    {
        let calls = calls.lock().await;
        let instructions = calls.last().unwrap()["instructions"].as_str().unwrap();
        assert!(instructions.contains("the owner confirmed the pending action"));
        assert!(instructions.contains("give every link"));
    }
    // A card the owner has since talked past is not confirmed by a plain yes
    // (it may answer another question), only by quoting its code.
    let (stale, _) = acks::request_tracked(
        &state.db,
        &chat,
        acks::Request {
            kind: "action",
            service: None,
            tool: Some("nyxid__delete_channel_bot"),
            arguments: Some(&json!({"bot_id": "old-bot"})),
            summary: "Delete channel bot 'old-bot'",
            platform: false,
        },
    )
    .await
    .unwrap();
    respond(
        &state,
        &agent_key,
        &event("Which keys do I have?", "7", "evt-3"),
        "evt_3",
    )
    .await;
    respond(&state, &agent_key, &event("yes", "7", "evt-4"), "evt_4").await;
    let status = |id: String| {
        let state = state.clone();
        let conversation = conversation.id.clone();
        async move {
            acks::history(&state.db, OWNER, &conversation)
                .await
                .unwrap()
                .into_iter()
                .find(|ack| ack.id == id)
                .unwrap()
                .status
        }
    };
    assert_eq!(status(stale.id.clone()).await, "pending");
    let code = acks::confirm_code(&stale.id);
    respond(
        &state,
        &agent_key,
        &event(&format!("no {code}"), "7", "evt-5"),
        "evt_5",
    )
    .await;
    assert_eq!(status(stale.id.clone()).await, "denied");
    server.abort();
}

#[tokio::test]
async fn relinking_a_bot_to_a_specialist_starts_that_agents_own_thread() {
    let (state, calls, server) = setup("nyxbot_relink").await;
    let (row, agent_key) = channel(&state, "gateway").await;
    let bound = put_binding(
        State(state.clone()),
        Path("bnd_relink".into()),
        bearer(&agent_key),
        Json(binding_body(&agent_key, OWNER)),
    )
    .await;
    assert_eq!(bound.status(), StatusCode::OK);
    let ensured = put_conversation(
        State(state.clone()),
        Path(("bnd_relink".into(), PARTITION.into())),
        bearer(&agent_key),
    )
    .await;
    assert_eq!(ensured.status(), StatusCode::OK);
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"owner_sender_ids": ["7"]}},
        )
        .await
        .unwrap();
    let first =
        body_text(respond(&state, &agent_key, &event("Hi", "7", "evt-1"), "evt_1").await).await;
    assert!(first.contains("Here is your answer"), "{first}");
    let (specialist, _) = crate::services::assistant_team_service::create_specialist(
        &state.db,
        &state.encryption_keys,
        OWNER,
        crate::services::assistant_team_service::CreateRequest {
            name: "support".into(),
            description: "Answer questions from the support chat".into(),
            display_name: None,
            persona: None,
            targets: Default::default(),
            account_read: false,
            specialty: None,
            created_by: "user",
        },
    )
    .await
    .unwrap()
    .unwrap();
    let changed = link(&state, OWNER, &row.id, &specialist).await.unwrap();
    assert_eq!(changed["changed"], true);
    let second =
        body_text(respond(&state, &agent_key, &event("Hi", "7", "evt-2"), "evt_2").await).await;
    assert!(second.contains("Here is your answer"), "{second}");
    assert_eq!(calls.lock().await.len(), 2);
    let threads: Vec<AssistantConversation> = state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find(doc! {"user_id": OWNER, "channel.nyxbot_channel_id": &row.id})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(threads.len(), 2);
    let (nyxbot_thread, specialist_thread): (Vec<_>, Vec<_>) =
        threads.iter().partition(|thread| !thread.is_subagent());
    assert_eq!(
        specialist_thread[0].agent_id.as_deref(),
        Some(specialist.id.as_str())
    );
    // The chat now maps to the specialist's thread; the NyxBot thread's late
    // updates stay in the app.
    let mapping = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"channel_id": &row.id, "partition": PARTITION})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        mapping.conversation_id.as_deref(),
        Some(specialist_thread[0].id.as_str())
    );
    assert_ne!(nyxbot_thread[0].id, specialist_thread[0].id);
    // Linking to the same agent again changes nothing.
    let unchanged = link(&state, OWNER, &row.id, &specialist).await.unwrap();
    assert_eq!(unchanged["changed"], false);
    server.abort();
}

async fn wait_for_event(state: &AppState, conversation_id: &str, needle: &str) -> String {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let messages = engine::messages(&state.db, OWNER, conversation_id, 100, None)
                .await
                .unwrap();
            if let Some(message) = messages
                .iter()
                .find(|message| message.role == "event" && message.text.contains(needle))
            {
                return message.text.clone();
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("the waiting chat was resumed")
}

fn bot_doc(platform: &str, label: &str) -> bson::Document {
    doc! {
        "_id": Uuid::new_v4().to_string(), "user_id": OWNER, "platform": platform,
        "label": label, "platform_bot_id": "123", "platform_bot_username": "helper_bot",
        "bot_token_encrypted": bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic, bytes: vec![],
        },
        "webhook_secret_hash": "hash", "webhook_registered": true, "status": "active",
        "is_active": true, "created_at": bson::DateTime::now(),
        "updated_at": bson::DateTime::now(),
    }
}

/// NyxBot hands out NyxID's setup page instead of asking for tokens or
/// pointing at Studio, and the bot the user creates there is linked and
/// reported without the user coming back to say so.
#[tokio::test]
async fn setup_links_create_bots_that_link_themselves_and_resume_the_chat() {
    let (state, calls, server) = setup("nyxbot_setup_link").await;
    let team = crate::services::assistant_team_service::ensure_nyxbot(&state.db, OWNER)
        .await
        .unwrap();
    let home = crate::services::assistant_team_service::home_thread(
        &state.db,
        &state.encryption_keys,
        &team,
    )
    .await
    .unwrap();
    let unknown = setup_link_tool(&state, OWNER, &home.id, "myspace", None, &team).await;
    assert!(matches!(unknown, Err(AppError::ValidationError(message)) if message.contains("lark")));
    // Without an administrator-configured creation manager, Telegram uses
    // the bot-token page.
    let (telegram, _) = setup_link_tool(&state, OWNER, &home.id, "telegram", Some("Helper"), &team)
        .await
        .unwrap();
    assert!(
        telegram["url"]
            .as_str()
            .unwrap()
            .ends_with("/channel-bots/connect/telegram?label=Helper"),
        "{telegram}"
    );
    let (lark, _) = setup_link_tool(&state, OWNER, &home.id, "lark", None, &team)
        .await
        .unwrap();
    assert!(
        lark["url"]
            .as_str()
            .unwrap()
            .ends_with("/channel-bots/connect/lark")
    );
    assert!(
        lark["note"]
            .as_str()
            .unwrap()
            .contains("Do not ask them to reply")
    );
    // The chat shows what it is waiting for.
    let titles = |items: Vec<WaitingItem>| -> Vec<String> {
        items.into_iter().map(|item| item.title).collect()
    };
    let waiting_now = titles(waiting(&state, OWNER, &home.id).await.unwrap());
    assert!(
        waiting_now.contains(&"Waiting for your Telegram bot to be created".to_owned())
            && waiting_now.contains(&"Waiting for your Lark bot to be created".to_owned()),
        "{waiting_now:?}"
    );
    // Nothing to link yet; a bot created before the link is never taken.
    let mut older = bot_doc("lark", "Older");
    older.insert(
        "created_at",
        bson::DateTime::from_chrono(Utc::now() - ChronoDuration::hours(1)),
    );
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(older)
        .await
        .unwrap();
    process_watches(&state).await.unwrap();
    assert_eq!(calls.lock().await.len(), 0);
    // The user finishes the setup page: NyxID links the new bot and resumes
    // the chat that asked.
    let created = bot_doc("lark", "Support desk");
    let bot_id = created.get_str("_id").unwrap().to_owned();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(created)
        .await
        .unwrap();
    process_watches(&state).await.unwrap();
    let channel = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"user_id": OWNER, "channel_bot_id": &bot_id})
        .await
        .unwrap()
        .expect("the new bot is linked");
    assert_eq!(channel.agent_id.as_deref(), Some(team.id.as_str()));
    let event = wait_for_event(&state, &home.id, "now linked to").await;
    // Untrusted names reach the model only as sanitized identifiers.
    assert!(
        event.contains("Supportdesk") && event.contains("owner-verification"),
        "{event}"
    );
    // The bot exists; now the chat waits for the owner to verify, and
    // stops once they have.
    let waiting_now = titles(waiting(&state, OWNER, &home.id).await.unwrap());
    assert!(
        !waiting_now.contains(&"Waiting for your Lark bot to be created".to_owned())
            && waiting_now.contains(
                &"Waiting for you to verify your Lark account with @helper_bot".to_owned()
            ),
        "{waiting_now:?}"
    );
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &channel.id},
            doc! {"$push": {"owner_sender_ids": "7"}},
        )
        .await
        .unwrap();
    let waiting_now = titles(waiting(&state, OWNER, &home.id).await.unwrap());
    assert_eq!(
        waiting_now,
        vec!["Waiting for your Telegram bot to be created"]
    );
    // Reconnecting a channel whose messages stopped arriving rebuilds it
    // from scratch, and its verified owner stays verified.
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &channel.id},
            doc! {"$set": {"delivery_status": "failing", "delivery_error": "refused_401"}},
        )
        .await
        .unwrap();
    let (rebuilt, _) = connect(&state, OWNER, Some(&home.id), &bot_id, &team)
        .await
        .unwrap();
    assert_ne!(rebuilt.id, channel.id);
    assert_eq!(rebuilt.owner_sender_ids, vec!["7".to_owned()]);
    assert_eq!(rebuilt.delivery_status, None);
    assert_eq!(delivery(&state, &channel.id).await.status, "disconnected");
    // Other threads wait for nothing.
    assert!(waiting(&state, OWNER, "other").await.unwrap().is_empty());
    // Linking happens once.
    process_watches(&state).await.unwrap();
    assert_eq!(
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .count_documents(doc! {"user_id": OWNER, "status": {"$ne": "disconnected"}})
            .await
            .unwrap(),
        1
    );
    server.abort();
}

/// A connect link a chat hands out resumes that chat when the user finishes
/// it; the user never replies "connected".
#[tokio::test]
async fn finished_connect_links_resume_the_chat_that_sent_them() {
    let (state, _, server) = setup("nyxbot_connect_watch").await;
    let team = crate::services::assistant_team_service::ensure_nyxbot(&state.db, OWNER)
        .await
        .unwrap();
    let home = crate::services::assistant_team_service::home_thread(
        &state.db,
        &state.encryption_keys,
        &team,
    )
    .await
    .unwrap();
    let link_id = Uuid::new_v4().to_string();
    let now = bson::DateTime::now();
    state
        .db
        .collection::<bson::Document>(crate::models::connect_link::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": &link_id, "user_id": OWNER, "service_slug": "api-github",
            "service_id": Uuid::new_v4().to_string(), "token_hash": "hash",
            "status": "pending", "created_at": now,
            "expires_at": bson::DateTime::from_chrono(Utc::now() + ChronoDuration::minutes(15)),
        })
        .await
        .unwrap();
    watch_connect_link(&state.db, OWNER, &home.id, &link_id)
        .await
        .unwrap();
    let pending = waiting(&state, OWNER, &home.id).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].kind, "connect_link");
    assert_eq!(
        pending[0].title,
        "Waiting for you to finish connecting api-github"
    );
    process_watches(&state).await.unwrap();
    assert!(
        engine::get(&state.db, OWNER, &home.id)
            .await
            .unwrap()
            .pending_events
            .is_empty()
    );
    state
        .db
        .collection::<bson::Document>(crate::models::connect_link::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &link_id},
            doc! {"$set": {"status": "completed"}},
        )
        .await
        .unwrap();
    process_watches(&state).await.unwrap();
    let event = wait_for_event(&state, &home.id, "finished connecting").await;
    assert!(event.contains("api-github") && event.contains("do not ask them to confirm"));
    assert!(waiting(&state, OWNER, &home.id).await.unwrap().is_empty());
    server.abort();
}

/// An inbound text message on a route; returns its NyxID message ID.
async fn inbound(
    state: &AppState,
    route_id: &str,
    chat: &str,
    status: &str,
    http_status: Option<i32>,
    age_secs: i64,
) -> String {
    inbound_of(state, route_id, chat, "text", status, http_status, age_secs).await
}

async fn inbound_of(
    state: &AppState,
    route_id: &str,
    chat: &str,
    content_type: &str,
    status: &str,
    http_status: Option<i32>,
    age_secs: i64,
) -> String {
    let id = Uuid::new_v4().to_string();
    let created_at = bson::DateTime::from_chrono(Utc::now() - ChronoDuration::seconds(age_secs));
    state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": &id, "conversation_id": route_id,
            "platform_conversation_id": chat, "sender_platform_id": "7", "user_id": OWNER,
            "direction": "inbound", "platform": "telegram-new", "content_type": content_type,
            "callback_status": status, "callback_http_status": http_status,
            "created_at": created_at,
        })
        .await
        .unwrap();
    id
}

fn admission(channel_id: &str, event_id: &str) -> NyxbotEvent {
    NyxbotEvent {
        id: Uuid::new_v4().to_string(),
        channel_id: channel_id.into(),
        user_id: OWNER.into(),
        partition: PARTITION.into(),
        event_id: event_id.into(),
        event_context_ciphertext: None,
        status: "completed".into(),
        conversation_id: None,
        turn_id: None,
        created_at: Utc::now(),
        expires_at: Utc::now() + ChronoDuration::hours(1),
    }
}

async fn delivery(state: &AppState, id: &str) -> NyxbotChannel {
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id": id})
        .await
        .unwrap()
        .unwrap()
}

/// A chat app whose messages stop reaching the agent never goes quiet
/// silently: the channel is flagged and the chat that set it up hears about
/// it once, in plain words.
#[tokio::test]
async fn lost_chat_app_messages_are_reported_to_the_agent_once() {
    let (state, _, server) = setup("nyxbot_delivery_health").await;
    let team = crate::services::assistant_team_service::ensure_nyxbot(&state.db, OWNER)
        .await
        .unwrap();
    let home = crate::services::assistant_team_service::home_thread(
        &state.db,
        &state.encryption_keys,
        &team,
    )
    .await
    .unwrap();
    let (row, _) = channel(&state, "gateway").await;
    let route_id = Uuid::new_v4().to_string();
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"route_id": &route_id, "source_conversation_id": &home.id,
            "created_at": bson::DateTime::from_chrono(Utc::now() - ChronoDuration::hours(3))}},
        )
        .await
        .unwrap();
    // A failure from before the first check's short look-back is not news.
    inbound(&state, &route_id, "7", "failed", Some(401), 2 * 3600).await;
    check_deliveries(&state).await.unwrap();
    assert_eq!(delivery(&state, &row.id).await.delivery_status, None);
    // The gateway refuses the owner's message: flagged, and the chat is told.
    inbound(&state, &route_id, "7", "failed", Some(401), 1).await;
    check_deliveries(&state).await.unwrap();
    let flagged = delivery(&state, &row.id).await;
    assert_eq!(flagged.delivery_status.as_deref(), Some("failing"));
    assert_eq!(flagged.delivery_error.as_deref(), Some("refused_401"));
    let notice = wait_for_event(&state, &home.id, "are not reaching").await;
    assert!(
        notice.contains("Telegram")
            && notice.contains("Agent Event Gateway refused it (HTTP 401)")
            && notice.contains("not something they did"),
        "{notice}"
    );
    let listed = list_tool(&state, OWNER).await.unwrap();
    assert_eq!(listed["channel_agents"][0]["delivery_status"], "failing");
    assert_eq!(
        listed["channel_agents"][0]["delivery_reason"],
        "the Agent Event Gateway refused it (HTTP 401)"
    );
    // More of the same raises no second notice.
    inbound(&state, &route_id, "7", "failed", None, 0).await;
    check_deliveries(&state).await.unwrap();
    assert_eq!(
        delivery(&state, &row.id).await.delivery_error.as_deref(),
        Some("undelivered")
    );
    // An accepted private message still in flight is not judged yet...
    inbound(&state, &route_id, "7", "delivered", None, 10).await;
    check_deliveries(&state).await.unwrap();
    assert_eq!(
        delivery(&state, &row.id).await.delivery_status.as_deref(),
        Some("failing")
    );
    // ...but a private text the gateway accepted and never passed on counts
    // as lost, while a photo (which it refuses with a 202) and a group
    // message it may filter on purpose say nothing.
    state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .delete_many(doc! {"conversation_id": &route_id})
        .await
        .unwrap();
    inbound(&state, &route_id, "-100", "delivered", None, 200).await;
    inbound(&state, &route_id, "7", "delivered", None, 180).await;
    inbound_of(&state, &route_id, "7", "image", "delivered", None, 170).await;
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"delivery_seen_at":
            bson::DateTime::from_chrono(Utc::now() - ChronoDuration::seconds(300))}},
        )
        .await
        .unwrap();
    check_deliveries(&state).await.unwrap();
    assert_eq!(
        delivery(&state, &row.id).await.delivery_error.as_deref(),
        Some("not_received")
    );
    // A newer group message does not make a failing channel look healthy.
    inbound(&state, &route_id, "-100", "delivered", None, 150).await;
    check_deliveries(&state).await.unwrap();
    assert_eq!(
        delivery(&state, &row.id).await.delivery_status.as_deref(),
        Some("failing")
    );
    // Admission is matched per message: another message's admission does
    // not vouch for this one.
    inbound(&state, &route_id, "7", "delivered", None, 130).await;
    state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .insert_one(admission(&row.id, &Uuid::new_v4().to_string()))
        .await
        .unwrap();
    let before = delivery(&state, &row.id).await.delivery_failed_at;
    check_deliveries(&state).await.unwrap();
    let still = delivery(&state, &row.id).await;
    assert_eq!(still.delivery_error.as_deref(), Some("not_received"));
    assert!(
        still.delivery_failed_at > before,
        "the newer lost message is recorded"
    );
    // Once a message reaches the agent again, the channel is healthy.
    let arrived = inbound(&state, &route_id, "7", "delivered", None, 0).await;
    let admitted = NyxbotEvent {
        id: Uuid::new_v4().to_string(),
        channel_id: row.id.clone(),
        user_id: OWNER.into(),
        partition: PARTITION.into(),
        event_id: arrived.clone(),
        event_context_ciphertext: None,
        status: "completed".into(),
        conversation_id: None,
        turn_id: None,
        created_at: Utc::now(),
        expires_at: Utc::now() + ChronoDuration::hours(1),
    };
    state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .insert_one(&admitted)
        .await
        .unwrap();
    check_deliveries(&state).await.unwrap();
    let healthy = delivery(&state, &row.id).await;
    assert_eq!(healthy.delivery_status.as_deref(), Some("ok"));
    assert_eq!(healthy.delivery_error, None);
    let notices = engine::messages(&state.db, OWNER, &home.id, 100, None)
        .await
        .unwrap()
        .into_iter()
        .filter(|message| message.role == "event" && message.text.contains("are not reaching"))
        .count();
    assert_eq!(notices, 1);
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
    for token in [
        sign("other-key", &digest),
        sign(&row.route_api_key_id, "00"),
    ] {
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

/// Start this state's change stream and NyxBot's live dispatch, and return
/// once the stream is delivering changes.
async fn start_live(state: &AppState) -> tokio::task::JoinHandle<()> {
    use crate::services::assistant_live::LiveEvent;
    let mut probe = state.assistant_live.subscribe();
    spawn_live_dispatch(state.clone());
    let runner = {
        let live = state.assistant_live.clone();
        let db = state.db.clone();
        tokio::spawn(async move { live.run(db).await })
    };
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            state
                .db
                .collection::<bson::Document>(CONVERSATIONS)
                .update_one(
                    doc! {"_id": "live-probe"},
                    doc! {"$set": {"user_id": "probe", "at": bson::DateTime::now()}},
                )
                .upsert(true)
                .await
                .unwrap();
            let deadline = tokio::time::sleep(Duration::from_millis(300));
            tokio::pin!(deadline);
            loop {
                tokio::select! {
                    event = probe.recv() => {
                        if matches!(event, Ok(LiveEvent::Conversation { ref id, .. }) if id == "live-probe") {
                            return;
                        }
                    }
                    _ = &mut deadline => break,
                }
            }
        }
    })
    .await
    .expect("the change stream opens");
    runner
}

/// With the change stream running, a finished connect link and a newly
/// active bot resume the waiting chat at once; no sweep runs here.
#[tokio::test]
async fn live_changes_resume_waiting_chats_without_the_sweep() {
    let (state, _, server) = setup("nyxbot_live_resume").await;
    let runner = start_live(&state).await;
    let team = crate::services::assistant_team_service::ensure_nyxbot(&state.db, OWNER)
        .await
        .unwrap();
    let home = crate::services::assistant_team_service::home_thread(
        &state.db,
        &state.encryption_keys,
        &team,
    )
    .await
    .unwrap();
    // A connect link the chat handed out.
    let link_id = Uuid::new_v4().to_string();
    state
        .db
        .collection::<bson::Document>(crate::models::connect_link::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": &link_id, "user_id": OWNER, "service_slug": "api-github",
            "service_id": Uuid::new_v4().to_string(), "token_hash": "hash",
            "status": "pending", "created_at": bson::DateTime::now(),
            "expires_at": bson::DateTime::from_chrono(Utc::now() + ChronoDuration::minutes(15)),
        })
        .await
        .unwrap();
    watch_connect_link(&state.db, OWNER, &home.id, &link_id)
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::connect_link::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &link_id},
            doc! {"$set": {"status": "completed"}},
        )
        .await
        .unwrap();
    wait_for_event(&state, &home.id, "finished connecting").await;
    // A setup link: the bot the user creates is linked the moment it is
    // saved, even while its webhook is still being verified.
    setup_link_tool(&state, OWNER, &home.id, "lark", None, &team)
        .await
        .unwrap();
    let mut created = bot_doc("lark", "Support desk");
    created.insert("status", "pending_webhook");
    let bot_id = created.get_str("_id").unwrap().to_owned();
    let linked = || async {
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .count_documents(doc! {"user_id": OWNER, "channel_bot_id": &bot_id})
            .await
            .unwrap()
    };
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(created)
        .await
        .unwrap();
    wait_for_event(&state, &home.id, "now linked to").await;
    assert_eq!(linked().await, 1);
    runner.abort();
    server.abort();
}

async fn frame(body: &mut axum::body::BodyDataStream) -> String {
    use futures::StreamExt;
    let bytes = tokio::time::timeout(Duration::from_secs(5), body.next())
        .await
        .expect("a frame")
        .expect("an open stream")
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// The browser's live stream carries only the owner's changes, as
/// identifiers, and asks for a resync when changes may have been missed.
#[tokio::test]
async fn the_live_stream_pushes_only_the_owners_changes() {
    use crate::services::assistant_live::LiveEvent;
    let (state, _, server) = setup("nyxbot_live_stream").await;
    // Without an open change stream, browsers are told to keep polling.
    let closed = crate::handlers::assistant_nyxagent::live(
        State(state.clone()),
        crate::test_utils::test_auth_user(OWNER),
    )
    .await
    .unwrap();
    assert_eq!(closed.status(), StatusCode::SERVICE_UNAVAILABLE);
    state.assistant_live.set_open_for_tests(true);
    let response = crate::handlers::assistant_nyxagent::live(
        State(state.clone()),
        crate::test_utils::test_auth_user(OWNER),
    )
    .await
    .unwrap();
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let mut body = response.into_body().into_data_stream();
    assert!(frame(&mut body).await.contains("event: ready"));
    state.assistant_live.publish(LiveEvent::Conversation {
        id: "nyxa-other".into(),
        user_id: "someone-else".into(),
        group_id: None,
        turn_id: None,
        messages: 1,
    });
    state.assistant_live.publish(LiveEvent::Conversation {
        id: "nyxa-mine".into(),
        user_id: OWNER.into(),
        group_id: Some("nyxg-1".into()),
        turn_id: Some("turn-1".into()),
        messages: 3,
    });
    let pushed = frame(&mut body).await;
    assert!(
        pushed.contains("event: conversation")
            && pushed.contains(r#""type":"conversation""#)
            && pushed.contains(r#""id":"nyxa-mine""#)
            && pushed.contains(r#""group_id":"nyxg-1""#)
            && pushed.contains(r#""turn_id":"turn-1""#)
            && pushed.contains(r#""messages":3"#)
            && !pushed.contains("nyxa-other"),
        "{pushed}"
    );
    state.assistant_live.publish(LiveEvent::Resync);
    assert!(frame(&mut body).await.contains("event: resync"));
    // One owner may hold only a few streams at once.
    let mut held = Vec::new();
    for _ in 1..crate::services::assistant_live::MAX_STREAMS_PER_OWNER {
        held.push(
            state
                .assistant_live
                .subscribe_owner(OWNER)
                .expect("under the cap"),
        );
    }
    let refused = crate::handlers::assistant_nyxagent::live(
        State(state.clone()),
        crate::test_utils::test_auth_user(OWNER),
    )
    .await
    .unwrap();
    assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        state
            .assistant_live
            .subscribe_owner("someone-else")
            .is_some(),
        "the cap is per owner"
    );
    drop(held);
    // The change stream dropping ends the browser's stream.
    state.assistant_live.set_open_for_tests(false);
    assert!(
        tokio::time::timeout(Duration::from_secs(5), futures::StreamExt::next(&mut body))
            .await
            .expect("the stream ends")
            .is_none()
    );
    server.abort();
}

async fn bot_inbound(state: &AppState, bot_id: &str, route_id: &str, status: &str) {
    state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": Uuid::new_v4().to_string(), "channel_bot_id": bot_id,
            "conversation_id": route_id, "platform_conversation_id": "7",
            "sender_platform_id": "7", "user_id": OWNER, "direction": "inbound",
            "platform": "lark", "content_type": "text", "callback_status": status,
            "created_at": bson::DateTime::now(),
        })
        .await
        .unwrap();
}

/// An owner who "already sent the code twice" learns why it did not work:
/// nothing arrived, or a route from an earlier setup took the chat, or the
/// message arrived without the code. A route taking the owner's chat is
/// reported to the agent with the fix.
#[tokio::test]
async fn owner_messages_taken_by_another_route_are_explained() {
    let (state, _, server) = setup("nyxbot_routed_elsewhere").await;
    let team = crate::services::assistant_team_service::ensure_nyxbot(&state.db, OWNER)
        .await
        .unwrap();
    let home = crate::services::assistant_team_service::home_thread(
        &state.db,
        &state.encryption_keys,
        &team,
    )
    .await
    .unwrap();
    let (row, _) = channel(&state, "direct").await;
    let route_id = Uuid::new_v4().to_string();
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"route_id": &route_id, "source_conversation_id": &home.id}},
        )
        .await
        .unwrap();
    let row = load_channel(&state, OWNER, &row.id).await.unwrap();
    refresh_link_code(&state, &row).await.unwrap();
    let row = load_channel(&state, OWNER, &row.id).await.unwrap();
    let hint = || async {
        status::verification_hint(&state, &load_channel(&state, OWNER, &row.id).await.unwrap())
            .await
            .unwrap()
            .unwrap_or_default()
    };
    assert!(hint().await.contains("has not received any message"));
    let waiting_now = waiting(&state, OWNER, &home.id).await.unwrap();
    assert!(
        waiting_now[0]
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("event subscription")),
        "{waiting_now:?}"
    );
    // A chat-specific route from an earlier setup takes the owner's chat.
    let old_route = Uuid::new_v4().to_string();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
        .insert_one(doc! {"_id": &old_route, "user_id": OWNER,
        "channel_bot_id": &row.channel_bot_id, "is_active": true})
        .await
        .unwrap();
    assert_eq!(other_routes(&state, &row).await.unwrap(), 1);
    bot_inbound(&state, &row.channel_bot_id, &old_route, "delivered").await;
    assert!(hint().await.contains("another route on this bot"));
    check_deliveries(&state).await.unwrap();
    assert_eq!(
        delivery(&state, &row.id).await.delivery_error.as_deref(),
        Some("routed_elsewhere")
    );
    let notice = wait_for_event(&state, &home.id, "are not reaching").await;
    assert!(
        notice.contains("different agent")
            && notice.contains("nyxid__delete_channel_route")
            && notice.contains("with their OK"),
        "{notice}"
    );
    // Once the chat reaches the agent without the code, that is what it says.
    bot_inbound(&state, &row.channel_bot_id, &route_id, "delivered").await;
    assert!(hint().await.contains("not the current code"));
    let listed = list_tool(&state, OWNER).await.unwrap();
    assert!(
        listed["channel_agents"][0]["inbound_hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("not the current code"))
    );
    server.abort();
}

/// Linking an existing bot answers the setup link the chat handed out, so
/// the chat stops showing that it waits for a new bot.
#[tokio::test]
async fn linking_an_existing_bot_ends_the_wait_for_a_new_one() {
    let (state, _, server) = setup("nyxbot_existing_bot_settles_setup").await;
    let team = crate::services::assistant_team_service::ensure_nyxbot(&state.db, OWNER)
        .await
        .unwrap();
    let home = crate::services::assistant_team_service::home_thread(
        &state.db,
        &state.encryption_keys,
        &team,
    )
    .await
    .unwrap();
    let mut existing = bot_doc("lark", "Office");
    existing.insert(
        "created_at",
        bson::DateTime::from_chrono(Utc::now() - ChronoDuration::days(3)),
    );
    let bot_id = existing.get_str("_id").unwrap().to_owned();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(existing)
        .await
        .unwrap();
    setup_link_tool(&state, OWNER, &home.id, "lark", None, &team)
        .await
        .unwrap();
    let kinds = |items: Vec<WaitingItem>| -> Vec<&'static str> {
        items.into_iter().map(|item| item.kind).collect()
    };
    assert_eq!(
        kinds(waiting(&state, OWNER, &home.id).await.unwrap()),
        vec!["channel_bot"]
    );
    connect_tool(&state, OWNER, &home.id, &bot_id, &team)
        .await
        .unwrap();
    assert_eq!(
        kinds(waiting(&state, OWNER, &home.id).await.unwrap()),
        vec!["owner_verification"]
    );
    server.abort();
}

/// The same Lark app registered as two NyxID bots: the app delivers events
/// to one Request URL only, so the owner is told which bot receives them and
/// the exact URL this one needs.
#[tokio::test]
async fn a_second_bot_for_the_same_app_is_named_with_the_url_to_use() {
    let (state, _, server) = setup("nyxbot_twin_bot").await;
    let (row, _) = channel(&state, "direct").await;
    let bots = state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME);
    let mut linked = bot_doc("lark", "Office new");
    linked.insert("_id", &row.channel_bot_id);
    linked.insert("app_id", "cli_same_app");
    bots.insert_one(linked).await.unwrap();
    let mut older = bot_doc("lark", "Office old");
    let older_id = older.get_str("_id").unwrap().to_owned();
    older.insert("app_id", "cli_same_app");
    older.insert("platform_bot_id", "other");
    bots.insert_one(older).await.unwrap();
    refresh_link_code(&state, &row).await.unwrap();
    let row = load_channel(&state, OWNER, &row.id).await.unwrap();
    let hint = status::verification_hint(&state, &row)
        .await
        .unwrap()
        .unwrap();
    let url = format!(
        "{}/api/v1/webhooks/channel/lark/{}",
        state.config.base_url.trim_end_matches('/'),
        row.channel_bot_id
    );
    assert!(
        hint.contains("Office old") && hint.contains(&older_id) && hint.contains(&url),
        "{hint}"
    );
    // An unrelated bot of another app is no twin.
    bots.update_one(
        doc! {"_id": &older_id},
        doc! {"$set": {"app_id": "cli_other_app"}},
    )
    .await
    .unwrap();
    let hint = status::verification_hint(&state, &row)
        .await
        .unwrap()
        .unwrap();
    assert!(
        hint.contains("has not received any message") && hint.contains(&url),
        "{hint}"
    );
    server.abort();
}

/// An organization's channel bot can be linked by the org's admins: found
/// by its label, routed with org-owned route and key through NyxID's relay
/// (never the per-person gateway), reaching the admin's agent only while
/// they stay an admin, and cleaned up as the org's on disconnect.
#[tokio::test]
async fn org_admins_link_org_bots_by_label_and_lose_them_with_their_role() {
    use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole};
    let (state, _, server) = setup("nyxbot_org_bot").await;
    let org = Uuid::new_v4().to_string();
    state
        .db
        .collection::<crate::models::user::User>(USERS)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    state
        .db
        .collection::<OrgMembership>(MEMBERSHIPS)
        .insert_one(crate::test_utils::test_membership(
            &org,
            OWNER,
            OrgRole::Admin,
            None,
        ))
        .await
        .unwrap();
    let team = crate::services::assistant_team_service::ensure_nyxbot(&state.db, OWNER)
        .await
        .unwrap();
    let home = crate::services::assistant_team_service::home_thread(
        &state.db,
        &state.encryption_keys,
        &team,
    )
    .await
    .unwrap();
    // A Telegram org bot: it must still use NyxID's relay.
    let mut bot = bot_doc("telegram", "Office NyxBot");
    bot.insert("user_id", &org);
    let bot_id = bot.get_str("_id").unwrap().to_owned();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(bot)
        .await
        .unwrap();
    // Found by label (any case) among the admin's bots and the org's.
    let found = resolve_bot_ref(&state, OWNER, "office nyxbot")
        .await
        .unwrap();
    assert_eq!(found.id, bot_id);
    let (value, _) = connect_tool(&state, OWNER, &home.id, &bot_id, &team)
        .await
        .unwrap();
    assert_eq!(value["channel_agent"]["org_id"], org.as_str());
    let row = active_for_bot(&state, OWNER, &bot_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.transport, "direct");
    assert_eq!(row.bot_owner_id.as_deref(), Some(org.as_str()));
    let route = state
        .db
        .collection::<crate::models::channel_conversation::ChannelConversation>(
            crate::models::channel_conversation::COLLECTION_NAME,
        )
        .find_one(doc! {"_id": row.route_id.as_deref().unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(route.user_id, org);
    assert_eq!(
        key_service::get_api_key(&state.db, &org, &row.route_api_key_id)
            .await
            .unwrap()
            .user_id,
        org
    );
    // Its messages reach the admin's agent through NyxID's relay.
    let body = json!({
        "message_id": "msg-org", "correlation_id": "jti-org", "platform": "telegram",
        "reply_token": "not-used", "agent": {"api_key_id": row.route_api_key_id, "name": "route"},
        "conversation": {"id": "route", "platform_id": "chat", "type": "private"},
        "sender": {"platform_id": "stranger"}, "content": {"type": "text", "text": "hello"},
        "timestamp": "2026-09-28T00:00:00Z",
    });
    let bytes = serde_json::to_vec(&body).unwrap();
    let post = |message: &'static str, jti: &'static str, bytes: Vec<u8>| {
        let token = crate::crypto::jwt::generate_relay_callback_token(
            &state.jwt_keys,
            &state.config,
            jti,
            &row.route_api_key_id,
            message,
            "telegram",
            &sha256_hex(&bytes),
        )
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-nyxid-callback-token", token.parse().unwrap());
        relay_callback(
            State(state.clone()),
            Path(row.id.clone()),
            headers,
            Bytes::from(bytes),
        )
    };
    assert_eq!(
        post("msg-org", "jti-org", bytes).await.status(),
        StatusCode::ACCEPTED
    );
    // NyxBot's own replies act as the org's route key, not the person's.
    let reply = direct_reply(&state, &row, &Uuid::new_v4().to_string(), "hi", None).await;
    assert!(
        !matches!(&reply, Err(AppError::NotFound(message)) if message == "API key not found")
            && !matches!(&reply, Err(AppError::Forbidden(_))),
        "org replies must use the org's route key"
    );
    // A member who does not administer the org cannot link it.
    let member = Uuid::new_v4().to_string();
    state
        .db
        .collection::<crate::models::user::User>(USERS)
        .insert_one(test_user(&member, UserType::Person))
        .await
        .unwrap();
    state
        .db
        .collection::<OrgMembership>(MEMBERSHIPS)
        .insert_one(crate::test_utils::test_membership(
            &org,
            &member,
            OrgRole::Member,
            None,
        ))
        .await
        .unwrap();
    assert!(matches!(
        accessible_bot(&state, &member, &bot_id).await,
        Err(AppError::ChannelBotNotFound(_))
    ));
    let set_role = |role: &'static str| {
        let state = state.clone();
        let org = org.clone();
        async move {
            state
                .db
                .collection::<OrgMembership>(MEMBERSHIPS)
                .update_one(
                    doc! {"org_user_id": &org, "member_user_id": OWNER},
                    doc! {"$set": {"role": role}},
                )
                .await
                .unwrap();
        }
    };
    let route_active = |route_id: String| {
        let state = state.clone();
        async move {
            state
                .db
                .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
                .count_documents(doc! {"_id": route_id, "is_active": true})
                .await
                .unwrap()
                > 0
        }
    };
    // Losing the admin role: the next message is refused, and the link is
    // released (the org's route and key removed) so other admins can link it.
    set_role("member").await;
    let body = json!({
        "message_id": "msg-org-2", "correlation_id": "jti-org-2", "platform": "telegram",
        "reply_token": "not-used", "agent": {"api_key_id": row.route_api_key_id, "name": "route"},
        "conversation": {"id": "route", "platform_id": "chat", "type": "private"},
        "sender": {"platform_id": "stranger"}, "content": {"type": "text", "text": "hi"},
        "timestamp": "2026-09-28T00:00:00Z",
    });
    let refused = post("msg-org-2", "jti-org-2", serde_json::to_vec(&body).unwrap()).await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    let lost = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(lost.status, "failed");
    assert_eq!(lost.last_error.as_deref(), Some("org_access_lost"));
    assert!(!route_active(row.route_id.clone().unwrap()).await);
    assert!(
        key_service::get_api_key(&state.db, &org, &row.route_api_key_id)
            .await
            .is_err()
    );
    // Nor can NyxBot still post into the org's bot.
    assert!(matches!(
        direct_reply(&state, &row, "msg-org", "hi", None).await,
        Err(AppError::Forbidden(_))
    ));
    // Disconnecting what is left is harmless.
    disconnect(&state, OWNER, &row.id).await.unwrap();
    // Promoted again, the admin can link the bot afresh; a later demotion is
    // noticed by the sweep even when no message arrives.
    set_role("admin").await;
    connect_tool(&state, OWNER, &home.id, &bot_id, &team)
        .await
        .unwrap();
    let again = active_for_bot(&state, OWNER, &bot_id)
        .await
        .unwrap()
        .unwrap();
    assert!(route_active(again.route_id.clone().unwrap()).await);
    set_role("member").await;
    check_deliveries(&state).await.unwrap();
    assert_eq!(
        load_channel(&state, OWNER, &again.id)
            .await
            .unwrap()
            .last_error
            .as_deref(),
        Some("org_access_lost")
    );
    assert!(!route_active(again.route_id.clone().unwrap()).await);
    server.abort();
}
