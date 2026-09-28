use super::*;
use crate::{
    models::{
        assistant_conversation::COLLECTION_NAME as CONVERSATIONS,
        assistant_message::AssistantMessage,
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
        user::{COLLECTION_NAME as USERS, UserType},
    },
    services::{
        assistant_agent_credential_service as credentials, assistant_authority_tests::connected,
        key_service,
    },
    test_utils::{connect_transaction_test_database, test_app_state, test_auth_user, test_user},
};
use axum::{
    Router,
    http::HeaderMap,
    response::{IntoResponse, Sse, sse::Event},
    routing::post,
};
use mongodb::bson::doc;
use std::{convert::Infallible, sync::Arc};
use tokio::sync::Mutex;

const OWNER: &str = "12345678-1234-4123-8123-123456789abc";

#[derive(Clone)]
struct Call {
    authorization: String,
    body: Value,
}
type Calls = Arc<Mutex<Vec<Call>>>;

/// A NyxAgent stand-in answering every turn with one completed reply.
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
        post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let sink = sink.clone();
            async move {
                sink.lock().await.push(Call {
                    authorization: headers["authorization"].to_str().unwrap().into(),
                    body,
                });
                let session = format!("conv_{}", Uuid::new_v4().simple());
                let response = format!("resp_{}_{}", &session[5..], Uuid::new_v4().simple());
                let stream = async_stream::stream! {
                    let completed = json!({
                        "type": "response.completed",
                        "sequence_number": 1,
                        "response": {
                            "id": response,
                            "conversation": {"id": session},
                            "status": "completed",
                            "output": [{"type": "message", "role": "assistant",
                                "content": [{"type": "output_text", "text": "Work finished"}]}],
                        },
                    });
                    yield Ok::<_, Infallible>(Event::default().data(completed.to_string()));
                };
                Sse::new(stream).into_response()
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

async fn idle_row(state: &AppState, id: &str) -> AssistantConversation {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let row = engine::get(&state.db, OWNER, id).await.unwrap();
            if row.active_turn.is_none() {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("turn settled")
}

async fn transcript(state: &AppState, id: &str) -> Vec<AssistantMessage> {
    engine::messages(&state.db, OWNER, id, 100, None)
        .await
        .unwrap()
}

async fn orchestrator(state: &AppState) -> (AssistantConversation, ChatAuthority) {
    let permit = state.direct_chat_limiter.try_acquire(OWNER).await.unwrap();
    let (row, _) = super::super::assistant_nyxagent::start_turn(
        state,
        test_auth_user(OWNER),
        &TurnStart::from(&engine::TurnRequest {
            conversation_id: None,
            text: "Research the topic with helpers".into(),
            model: None,
            access_mode: None,
        }),
        Some(super::super::assistant_nyxagent::SERVER_TURN_POLICY),
        permit,
    )
    .await
    .unwrap();
    let row = idle_row(state, &row.id).await;
    let chat = acks::for_key(&state.db, OWNER, Some(&row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    (row, chat)
}

async fn spawn(state: &AppState, chat: &ChatAuthority, args: Value) -> Value {
    let (value, error) = execute_tool(state, chat, "nyxid__spawn_subagent", &args).await;
    assert!(!error, "{value}");
    value
}

#[tokio::test]
async fn spawned_subagent_works_with_its_own_key_and_wakes_the_orchestrator_with_its_report() {
    let (state, calls, server) = setup("team_spawn_report").await;
    let (orchestrator, chat) = orchestrator(&state).await;
    let result = spawn(
        &state,
        &chat,
        json!({"name": "researcher", "charter": "Summarize the release notes",
            "task": "Summarize the 0.31 release notes"}),
    )
    .await;
    assert_eq!(result["task"]["status"], "started");
    let sub_id = result["subagent"]["id"].as_str().unwrap().to_owned();
    // The subagent settles, then its report wakes the idle orchestrator.
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let events = transcript(&state, &orchestrator.id)
                .await
                .into_iter()
                .filter(|message| message.role == "event")
                .count();
            let row = engine::get(&state.db, OWNER, &orchestrator.id).await.unwrap();
            if events == 1 && row.active_turn.is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("orchestrator woken");
    let subagent = idle_row(&state, &sub_id).await;
    assert_eq!(subagent.role, AgentRole::Subagent);
    assert_eq!(subagent.team_id.as_deref(), Some(orchestrator.id.as_str()));
    let sub_messages = transcript(&state, &sub_id).await;
    assert_eq!(sub_messages[0].role, "orchestrator");
    assert_eq!(sub_messages[1].text, "Work finished");
    let event = transcript(&state, &orchestrator.id)
        .await
        .into_iter()
        .find(|message| message.role == "event")
        .unwrap();
    assert!(event.text.contains("Subagent researcher replied"), "{}", event.text);
    let calls = calls.lock().await;
    assert_eq!(calls.len(), 3);
    let sub_key =
        credentials::load_for_conversation(&state.db, &state.encryption_keys, OWNER, &sub_id)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        calls[1].authorization,
        format!("Bearer {}", sub_key.raw_key.as_str())
    );
    assert_ne!(calls[0].authorization, calls[1].authorization);
    let instructions = calls[1].body["instructions"].as_str().unwrap();
    assert!(instructions.starts_with(engine::SUBAGENT_PROMPT));
    assert!(instructions.contains("Summarize the release notes"));
    assert!(
        calls[2].body["input"]
            .as_str()
            .unwrap()
            .contains("Subagent researcher replied")
    );
    // The orchestrator's next instructions list its team.
    assert!(
        calls[2].body["instructions"]
            .as_str()
            .unwrap()
            .contains("Your live subagents")
    );
    // The subagent key is restricted to its (empty) grants.
    let key = key_service::get_api_key(&state.db, OWNER, &sub_key.api_key_id)
        .await
        .unwrap();
    assert!(!key.allow_all_services && key.allowed_service_ids.is_empty());
    assert_eq!(key.scopes, "proxy");
    server.abort();
}

#[tokio::test]
async fn subagents_cannot_use_team_tools_and_destroyed_subagents_are_read_only() {
    let (state, _, server) = setup("team_destroy").await;
    let (orchestrator, chat) = orchestrator(&state).await;
    let result = spawn(
        &state,
        &chat,
        json!({"name": "mailer", "charter": "Draft emails"}),
    )
    .await;
    let sub_id = result["subagent"]["id"].as_str().unwrap().to_owned();
    let sub = engine::get(&state.db, OWNER, &sub_id).await.unwrap();
    let sub_chat = acks::for_key(&state.db, OWNER, Some(&sub.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    for name in assistant_team_tools::TOOL_NAMES {
        let (value, error) =
            execute_tool(&state, &sub_chat, &format!("nyxid__{name}"), &json!({})).await;
        assert!(error, "{name}");
        assert_eq!(value["error"], "orchestrator_only", "{name}");
    }
    let (value, error) = execute_tool(
        &state,
        &chat,
        "nyxid__destroy_subagent",
        &json!({"subagent": "mailer"}),
    )
    .await;
    assert!(!error, "{value}");
    assert!(
        key_service::get_api_key(&state.db, OWNER, &sub.credential_api_key_id)
            .await
            .is_err()
    );
    assert!(
        credentials::load_for_conversation(&state.db, &state.encryption_keys, OWNER, &sub_id)
            .await
            .unwrap()
            .is_none()
    );
    let result = engine::begin_turn(
        &state.db,
        OWNER,
        &engine::TurnRequest {
            conversation_id: Some(sub_id.clone()),
            text: "are you there?".into(),
            model: None,
            access_mode: None,
        },
        &state.encryption_keys,
    )
    .await;
    assert!(matches!(result, Err(AppError::Conflict(_))));
    // Read-only: still listed and readable, not messageable.
    let (list, _) = execute_tool(
        &state,
        &chat,
        "nyxid__list_subagents",
        &json!({"include_destroyed": true}),
    )
    .await;
    assert_eq!(list["subagents"][0]["status"], "destroyed");
    let (value, error) = execute_tool(
        &state,
        &chat,
        "nyxid__message_subagent",
        &json!({"subagent": "mailer", "text": "hi"}),
    )
    .await;
    assert!(error, "{value}");
    assert!(engine::history_page(&state.db, OWNER, &sub_id, 10, None).await.is_ok());
    // The name can be reused by a new subagent.
    spawn(
        &state,
        &chat,
        json!({"name": "mailer", "charter": "Draft emails again"}),
    )
    .await;
    let _ = orchestrator;
    server.abort();
}

#[tokio::test]
async fn spawn_respects_owner_limits_and_grants_resolve_only_visible_services() {
    let (state, _, server) = setup("team_limits").await;
    let (_, chat) = orchestrator(&state).await;
    let github = connected(&state.db, OWNER, "github", "https://api.github.com").await;
    let other = connected(&state.db, "someone-else", "private", "https://example.com").await;
    let (value, error) = execute_tool(
        &state,
        &chat,
        "nyxid__spawn_subagent",
        &json!({"name": "x", "charter": "c", "services": ["does-not-exist"]}),
    )
    .await;
    assert!(error, "{value}");
    let (value, error) = execute_tool(
        &state,
        &chat,
        "nyxid__spawn_subagent",
        &json!({"name": "x", "charter": "c", "services": [other]}),
    )
    .await;
    assert!(error, "{value}");
    let created = spawn(
        &state,
        &chat,
        json!({"name": "coder", "charter": "Review PRs", "services": ["github"]}),
    )
    .await;
    assert_eq!(created["subagent"]["services"], json!(["github"]));
    let sub = engine::get(
        &state.db,
        OWNER,
        created["subagent"]["id"].as_str().unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(sub.grants.service_ids, vec![github.clone()]);
    let key = key_service::get_api_key(&state.db, OWNER, &sub.credential_api_key_id)
        .await
        .unwrap();
    assert_eq!(key.allowed_service_ids, vec![github.clone()]);
    let (value, error) = execute_tool(
        &state,
        &chat,
        "nyxid__revoke_subagent",
        &json!({"subagent": "coder", "services": ["github"], "account_read": true}),
    )
    .await;
    assert!(!error, "{value}");
    let key = key_service::get_api_key(&state.db, OWNER, &sub.credential_api_key_id)
        .await
        .unwrap();
    assert!(key.allowed_service_ids.is_empty());
    let (_, error) = execute_tool(
        &state,
        &chat,
        "nyxid__grant_subagent",
        &json!({"subagent": "coder", "services": ["github"], "account_read": true}),
    )
    .await;
    assert!(!error);
    let key = key_service::get_api_key(&state.db, OWNER, &sub.credential_api_key_id)
        .await
        .unwrap();
    assert_eq!(key.allowed_service_ids, vec![github]);
    assert!(key.scopes.contains(crate::mw::auth::ASSISTANT_ACCOUNT_SCOPE));
    assert!(!key.allow_all_services);
    settings::update(
        &state.db,
        OWNER,
        settings::Update {
            max_live_subagents: Some(1),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (value, error) = execute_tool(
        &state,
        &chat,
        "nyxid__spawn_subagent",
        &json!({"name": "second", "charter": "c"}),
    )
    .await;
    assert!(error);
    assert_eq!(value["error"], "limit_reached");
    assert_eq!(value["limit"], 1);
    for invalid in [
        settings::Update {
            max_concurrent_subagent_turns: Some(0),
            ..Default::default()
        },
        settings::Update {
            max_concurrent_subagent_turns: Some(9),
            ..Default::default()
        },
        settings::Update {
            max_live_subagents: Some(33),
            ..Default::default()
        },
    ] {
        assert!(settings::update(&state.db, OWNER, invalid).await.is_err());
    }
    server.abort();
}

#[tokio::test]
async fn permission_requests_reach_the_orchestrator_and_its_decision_resumes_the_subagent() {
    let (state, _, server) = setup("team_permissions").await;
    let (orchestrator, chat) = orchestrator(&state).await;
    let github = connected(&state.db, OWNER, "github", "https://api.github.com").await;
    let created = spawn(
        &state,
        &chat,
        json!({"name": "coder", "charter": "Review PRs"}),
    )
    .await;
    let sub_id = created["subagent"]["id"].as_str().unwrap().to_owned();
    // The user talks to the subagent directly; keep that turn live.
    let sub = engine::begin_turn(
        &state.db,
        OWNER,
        &engine::TurnRequest {
            conversation_id: Some(sub_id.clone()),
            text: "Please review my open GitHub PRs".into(),
            model: None,
            access_mode: None,
        },
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let sub_chat = acks::for_key(&state.db, OWNER, Some(&sub.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let (refusal, request) =
        acks::service_gate(&state.db, &sub_chat, &github, "github", "GitHub", false)
            .await
            .unwrap()
            .unwrap();
    let request = request.expect("a new request");
    assert_eq!(request.decider, "orchestrator");
    assert!(
        request
            .request_excerpt
            .as_deref()
            .unwrap()
            .contains("Please review my open GitHub PRs")
    );
    permission_requested(&state, &sub_chat, &request).await;
    // The idle orchestrator is woken with the request.
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if transcript(&state, &orchestrator.id)
                .await
                .iter()
                .any(|message| message.role == "event" && message.text.contains(&request.id))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("orchestrator woken with the request");
    idle_row(&state, &orchestrator.id).await;
    // Another team cannot decide it; its own orchestrator can.
    let (other, _) = {
        let permit = state.direct_chat_limiter.try_acquire(OWNER).await.unwrap();
        super::super::assistant_nyxagent::start_turn(
            &state,
            test_auth_user(OWNER),
            &TurnStart::from(&engine::TurnRequest {
                conversation_id: None,
                text: "Another team".into(),
                model: None,
                access_mode: None,
            }),
            Some(super::super::assistant_nyxagent::SERVER_TURN_POLICY),
            permit,
        )
        .await
        .unwrap()
    };
    let other = idle_row(&state, &other.id).await;
    let other_chat = acks::for_key(&state.db, OWNER, Some(&other.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let args = json!({"request_id": refusal["acknowledgement_id"], "decision": "allow",
        "reason": "The user asked to review their PRs"});
    let (_, error) =
        execute_tool(&state, &other_chat, "nyxid__decide_permission", &args).await;
    assert!(error);
    let (value, error) = execute_tool(&state, &chat, "nyxid__decide_permission", &args).await;
    assert!(!error, "{value}");
    let saved = engine::get(&state.db, OWNER, &sub_id).await.unwrap();
    assert_eq!(saved.grants.service_ids, vec![github.clone()]);
    assert!(
        saved
            .pending_events
            .iter()
            .any(|event| event.kind == "permission_decided" && event.text.contains("allowed"))
    );
    let key = key_service::get_api_key(&state.db, OWNER, &saved.credential_api_key_id)
        .await
        .unwrap();
    assert_eq!(key.allowed_service_ids, vec![github.clone()]);
    assert!(
        acks::service_gate(&state.db, &sub_chat, &github, "github", "GitHub", false)
            .await
            .unwrap()
            .is_none()
    );
    let ack = acks::history(&state.db, OWNER, &sub_id).await.unwrap();
    assert_eq!(ack[0].decided_by.as_deref(), Some("orchestrator"));
    assert!(ack[0].reason.as_deref().unwrap().contains("review their PRs"));
    server.abort();
}

#[tokio::test]
async fn loop_guards_and_direct_chats_never_wake_the_orchestrator() {
    let (state, calls, server) = setup("team_guards").await;
    let (orchestrator, chat) = orchestrator(&state).await;
    let created = spawn(&state, &chat, json!({"name": "helper", "charter": "Help"})).await;
    let sub_id = created["subagent"]["id"].as_str().unwrap().to_owned();
    let before = Utc::now();
    // A direct user turn on the subagent settles without waking the orchestrator.
    let permit = state.direct_chat_limiter.try_acquire(OWNER).await.unwrap();
    super::super::assistant_nyxagent::start_turn(
        &state,
        test_auth_user(OWNER),
        &TurnStart::from(&engine::TurnRequest {
            conversation_id: Some(sub_id.clone()),
            text: "Draft the launch post".into(),
            model: None,
            access_mode: None,
        }),
        Some(super::super::assistant_nyxagent::SERVER_TURN_POLICY),
        permit,
    )
    .await
    .unwrap();
    idle_row(&state, &sub_id).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let row = engine::get(&state.db, OWNER, &orchestrator.id).await.unwrap();
    assert!(row.pending_events.is_empty() && row.active_turn.is_none());
    assert_eq!(calls.lock().await.len(), 2);
    let note = team::direct_chats_note(&state.db, OWNER, &orchestrator.id, before)
        .await
        .unwrap();
    assert!(note.contains("to helper") && note.contains("Draft the launch post"));
    // After three consecutive event turns an orchestrator waits for the user.
    state
        .db
        .collection::<bson_doc::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &orchestrator.id},
            doc! {"$set": {"event_streak": team::MAX_EVENT_STREAK}},
        )
        .await
        .unwrap();
    engine::push_events(
        &state.db,
        OWNER,
        &orchestrator.id,
        vec![team::event("message", "note".into(), None)],
    )
    .await
    .unwrap();
    wake(&state, OWNER, &orchestrator.id).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let row = engine::get(&state.db, OWNER, &orchestrator.id).await.unwrap();
    assert!(row.active_turn.is_none());
    assert_eq!(row.pending_events.len(), 1);
    assert_eq!(calls.lock().await.len(), 2);
    // A user message resets the streak and carries the queued event.
    let permit = state.direct_chat_limiter.try_acquire(OWNER).await.unwrap();
    super::super::assistant_nyxagent::start_turn(
        &state,
        test_auth_user(OWNER),
        &TurnStart::from(&engine::TurnRequest {
            conversation_id: Some(orchestrator.id.clone()),
            text: "Any news?".into(),
            model: None,
            access_mode: None,
        }),
        Some(super::super::assistant_nyxagent::SERVER_TURN_POLICY),
        permit,
    )
    .await
    .unwrap();
    let row = idle_row(&state, &orchestrator.id).await;
    assert_eq!(row.event_streak, 0);
    assert!(row.pending_events.is_empty());
    let calls = calls.lock().await;
    let instructions = calls[2].body["instructions"].as_str().unwrap();
    assert!(instructions.contains("NyxID events since your previous turn"));
    assert!(instructions.contains("to helper"), "{instructions}");
    server.abort();
}

#[tokio::test]
async fn deleting_an_orchestrator_deletes_its_team_and_idle_subagents_are_swept() {
    let (state, _, server) = setup("team_delete").await;
    let (orchestrator, chat) = orchestrator(&state).await;
    let mut keys = Vec::new();
    for name in ["one", "two"] {
        let created = spawn(&state, &chat, json!({"name": name, "charter": "c"})).await;
        let row = engine::get(
            &state.db,
            OWNER,
            created["subagent"]["id"].as_str().unwrap(),
        )
        .await
        .unwrap();
        keys.push(row.credential_api_key_id);
    }
    // The idle sweep destroys only stale subagents.
    state
        .db
        .collection::<bson_doc::Document>(CONVERSATIONS)
        .update_one(
            doc! {"agent_name": "one"},
            doc! {"$set": {"updated_at": mongodb::bson::DateTime::from_chrono(
                Utc::now() - chrono::Duration::days(team::IDLE_DESTROY_DAYS + 1))}},
        )
        .await
        .unwrap();
    assert_eq!(team::sweep_idle(&state.db).await.unwrap(), 1);
    let summaries = team::summaries(&state.db, OWNER, &orchestrator.id, true, 0)
        .await
        .unwrap();
    assert_eq!(
        summaries
            .iter()
            .filter(|row| row.status == "destroyed")
            .count(),
        1
    );
    let rows = engine::delete(&state.db, OWNER, &orchestrator.id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);
    for key in keys {
        assert!(key_service::get_api_key(&state.db, OWNER, &key).await.is_err());
    }
    assert_eq!(
        state
            .db
            .collection::<bson_doc::Document>(CONVERSATIONS)
            .count_documents(doc! {"user_id": OWNER})
            .await
            .unwrap(),
        0
    );
    server.abort();
}

use mongodb::bson as bson_doc;
