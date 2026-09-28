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
        owner_sender_ids: Vec::new(),
        link_code_hash: None,
        link_code_expires_at: None,
        source_conversation_id: None,
        agent_id: None,
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
    // An ordinary message does not decide it.
    assert_eq!(confirmation("what does that do?"), None);
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
    let calls = calls.lock().await;
    let instructions = calls.last().unwrap()["instructions"].as_str().unwrap();
    assert!(instructions.contains("the owner confirmed the pending action"));
    assert!(instructions.contains("give every link"));
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
    // Linking happens once.
    process_watches(&state).await.unwrap();
    assert_eq!(
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .count_documents(doc! {"user_id": OWNER})
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
