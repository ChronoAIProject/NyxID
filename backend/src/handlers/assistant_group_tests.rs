use super::*;
use crate::{
    models::{
        assistant_group::HOPS_PER_MESSAGE,
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
        user::{COLLECTION_NAME as USERS, UserType},
    },
    services::{assistant_team_service::CreateRequest, key_service},
    test_utils::{connect_transaction_test_database, test_app_state, test_auth_user, test_user},
};
use axum::{
    Router,
    response::{IntoResponse, Sse, sse::Event},
    routing::post as route_post,
};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use uuid::Uuid;

const OWNER: &str = "12345678-1234-4123-8123-1234567890cd";

type Calls = Arc<Mutex<Vec<Value>>>;

/// A NyxAgent stand-in that answers as whichever member is speaking.
fn reply_for(body: &Value) -> String {
    let instructions = body["instructions"].as_str().unwrap_or_default();
    let input = body["input"].as_str().unwrap_or_default();
    if instructions.contains("You are researcher in the group chat") {
        if input.contains("ping-pong") || input.contains("your turn") {
            "@NyxBot your turn".into()
        } else {
            "Researcher summary ready.".into()
        }
    } else if instructions.contains("You are NyxBot in the group chat") {
        if input.contains("ping-pong") || input.contains("your turn") {
            "@researcher your turn".into()
        } else if input.contains("ask the researcher") {
            "Handing over: @researcher please summarize.".into()
        } else {
            "NyxBot here.".into()
        }
    } else {
        "Work finished".into()
    }
}

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
        route_post(move |Json(body): Json<Value>| {
            let sink = sink.clone();
            async move {
                let text = reply_for(&body);
                sink.lock().await.push(body);
                let session = format!("conv_{}", Uuid::new_v4().simple());
                let response = format!("resp_{}_{}", &session[5..], Uuid::new_v4().simple());
                let stream = async_stream::stream! {
                    let completed = json!({
                        "type": "response.completed", "sequence_number": 1,
                        "response": {"id": response, "conversation": {"id": session},
                            "status": "completed", "output": [{"type": "message",
                            "role": "assistant", "content": [{"type": "output_text",
                            "text": text}]}]},
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

async fn researcher(state: &AppState) -> AssistantAgent {
    team::create_specialist(
        &state.db,
        &state.encryption_keys,
        OWNER,
        CreateRequest {
            name: "researcher".into(),
            description: "Summarizes notes".into(),
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
    .unwrap()
    .0
}

/// Wait until the transcript has `count` messages and nobody is working.
async fn settled(state: &AppState, group_id: &str, count: usize) -> Vec<GroupMessage> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let rows = groups::messages(&state.db, OWNER, group_id, 200, None)
            .await
            .unwrap();
        let working = groups::working(&state.db, OWNER, group_id).await.unwrap();
        let group = groups::get(&state.db, OWNER, group_id).await.unwrap();
        if rows.len() >= count && working.is_empty() && group.pending_agent_ids.is_empty() {
            return rows;
        }
        if std::time::Instant::now() > deadline {
            panic!(
                "group not settled: want {count}, have {} {:?}; working {:?}; pending {:?}; hops {}",
                rows.len(),
                rows.iter()
                    .map(|row| format!(
                        "{}:{}",
                        row.agent_name.clone().unwrap_or(row.role.clone()),
                        row.text
                    ))
                    .collect::<Vec<_>>(),
                working,
                group.pending_agent_ids,
                group.hops_remaining
            );
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn agent_texts(rows: &[GroupMessage]) -> Vec<(String, String)> {
    rows.iter()
        .filter(|row| row.role == "agent")
        .map(|row| (row.agent_name.clone().unwrap_or_default(), row.text.clone()))
        .collect()
}

#[tokio::test]
async fn group_messages_reach_the_lead_or_the_mentioned_members_and_hand_offs_are_bounded() {
    let (state, _, server) = setup("group_routing").await;
    let nyxbot = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
    let researcher = researcher(&state).await;
    let (status, Json(created)) = create_group(
        State(state.clone()),
        test_auth_user(OWNER),
        Json(CreateGroupRequest {
            name: "Launch team".into(),
            member_agent_ids: vec![researcher.id.clone(), nyxbot.id.clone()],
        }),
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::CREATED);
    // NyxBot leads when it is a member.
    assert_eq!(created.lead_agent_id, nyxbot.id);
    let id = created.id.clone();
    // No mention: the lead answers.
    let (_, Json(posted)) = post_message(
        State(state.clone()),
        test_auth_user(OWNER),
        Path(id.clone()),
        Json(PostMessageRequest {
            text: "hello team".into(),
        }),
    )
    .await
    .unwrap();
    assert_eq!(posted["addressed_agent_ids"], json!([nyxbot.id]));
    let rows = settled(&state, &id, 3).await;
    assert_eq!(
        agent_texts(&rows),
        vec![("NyxBot".to_owned(), "NyxBot here.".to_owned())]
    );
    // A mention reaches only that member.
    post(&state, OWNER, &id, "@researcher summarize the notes", None)
        .await
        .unwrap();
    let rows = settled(&state, &id, 5).await;
    assert_eq!(
        agent_texts(&rows).last().unwrap(),
        &(
            "researcher".to_owned(),
            "Researcher summary ready.".to_owned()
        )
    );
    // The lead hands work to a member by mentioning it.
    post(&state, OWNER, &id, "ask the researcher for a summary", None)
        .await
        .unwrap();
    let rows = settled(&state, &id, 8).await;
    let texts = agent_texts(&rows);
    assert_eq!(
        texts[texts.len() - 2..].to_vec(),
        vec![
            (
                "NyxBot".to_owned(),
                "Handing over: @researcher please summarize.".to_owned()
            ),
            (
                "researcher".to_owned(),
                "Researcher summary ready.".to_owned()
            ),
        ]
    );
    // Members mentioning each other stop after the hand-off budget.
    let before = rows.len();
    post(&state, OWNER, &id, "ping-pong @researcher", None)
        .await
        .unwrap();
    let expected = before + 1 + 1 + HOPS_PER_MESSAGE as usize;
    let rows = settled(&state, &id, expected).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let rows_after = groups::messages(&state.db, OWNER, &id, 200, None)
        .await
        .unwrap();
    assert_eq!(rows_after.len(), rows.len());
    assert_eq!(rows.len(), expected);
    // Member threads are the group's: never listed as the agents' threads.
    for agent in [&nyxbot, &researcher] {
        let threads = team::threads(&state.db, agent, 100).await.unwrap();
        assert!(threads.iter().all(|thread| thread.group_id.is_none()));
    }
    // Each member speaks with its own key: the researcher's has no services.
    let thread = groups::member_thread(&state.db, OWNER, &id, &researcher.id)
        .await
        .unwrap()
        .unwrap();
    let key = key_service::get_api_key(&state.db, OWNER, &thread.credential_api_key_id)
        .await
        .unwrap();
    assert!(!key.allow_all_services && key.allowed_service_ids.is_empty());
    server.abort();
}

#[tokio::test]
async fn groups_validate_members_and_delete_with_their_member_threads() {
    let (state, _, server) = setup("group_lifecycle").await;
    let nyxbot = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
    let researcher = researcher(&state).await;
    for members in [vec![], vec![Uuid::new_v4().to_string()]] {
        let result = groups::create(&state.db, OWNER, "Team", &members, "user").await;
        assert!(matches!(result, Err(AppError::ValidationError(_))));
    }
    let group = groups::create(
        &state.db,
        OWNER,
        "Team",
        std::slice::from_ref(&researcher.id),
        "user",
    )
    .await
    .unwrap();
    // Without NyxBot, the first member leads.
    assert_eq!(group.lead_agent_id, researcher.id);
    let group = groups::update(
        &state.db,
        OWNER,
        &group.id,
        Some("Research"),
        Some(&[researcher.id.clone(), nyxbot.id.clone()]),
    )
    .await
    .unwrap();
    assert_eq!(group.name, "Research");
    assert_eq!(group.lead_agent_id, nyxbot.id);
    let notices: Vec<String> = groups::messages(&state.db, OWNER, &group.id, 50, None)
        .await
        .unwrap()
        .into_iter()
        .filter(|row| row.role == "notice")
        .map(|row| row.text)
        .collect();
    assert!(notices.iter().any(|text| text.contains("NyxBot joined")));
    post(&state, OWNER, &group.id, "hello", None).await.unwrap();
    settled(&state, &group.id, 4).await;
    let thread = groups::member_thread(&state.db, OWNER, &group.id, &nyxbot.id)
        .await
        .unwrap()
        .unwrap();
    // Another owner cannot see it.
    assert!(
        get_group(
            State(state.clone()),
            test_auth_user(&Uuid::new_v4().to_string()),
            Path(group.id.clone()),
        )
        .await
        .is_err()
    );
    delete_group(
        State(state.clone()),
        test_auth_user(OWNER),
        Path(group.id.clone()),
    )
    .await
    .unwrap();
    assert!(groups::get(&state.db, OWNER, &group.id).await.is_err());
    assert!(
        groups::messages(&state.db, OWNER, &group.id, 50, None)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(engine::get(&state.db, OWNER, &thread.id).await.is_err());
    assert!(
        key_service::get_api_key(&state.db, OWNER, &thread.credential_api_key_id)
            .await
            .map(|key| !key.is_active)
            .unwrap_or(true)
    );
    server.abort();
}

#[tokio::test]
async fn mentions_count_only_at_a_word_start() {
    let (state, _, server) = setup("group_mentions").await;
    let nyxbot = team::ensure_nyxbot(&state.db, OWNER).await.unwrap();
    let researcher = researcher(&state).await;
    let members = vec![nyxbot.clone(), researcher.clone()];
    assert_eq!(
        groups::mentions("mail me at team@researcher.dev", &members),
        Vec::<String>::new()
    );
    assert_eq!(
        groups::mentions(
            "(@Researcher) and @nyxbot, then @researcher again",
            &members
        ),
        vec![researcher.id.clone(), nyxbot.id.clone()]
    );
    assert_eq!(
        groups::mentions("@nobody here", &members),
        Vec::<String>::new()
    );
    server.abort();
}
