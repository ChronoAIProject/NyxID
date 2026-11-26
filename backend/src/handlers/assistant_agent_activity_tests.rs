use super::*;
use crate::{
    crypto::jwt,
    models::{
        assistant_acknowledgement::COLLECTION_NAME as ACKS,
        assistant_conversation::COLLECTION_NAME as CONVERSATIONS,
    },
    services::{feature_flag_service as flags, key_service, org_agent_tests::Fixture},
    test_utils::test_auth_user,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::Request,
};
use mongodb::bson::{DateTime as BsonDateTime, Document, doc};
use tower::ServiceExt;

const PATH: &str = "/api/v1/assistant/nyxagent/agents/activity";

fn bearer(state: &AppState, actor: &str) -> String {
    let token = jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(actor).unwrap(),
        "read write",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    format!("Bearer {token}")
}

async fn snapshot(app: &Router, header: (&str, &str)) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(PATH)
                .header(header.0, header.1)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 256 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn set_turn(state: &AppState, row: &AssistantConversation, facts: Document) {
    let mut turn = doc! {"turn_id": Uuid::new_v4().to_string(), "started_at": BsonDateTime::now()};
    turn.extend(facts);
    let result = state
        .db
        .collection::<Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &row.id, "user_id": &row.user_id},
            doc! {"$set": {"active_turn": turn}},
        )
        .await
        .unwrap();
    assert_eq!(result.matched_count, 1);
}

fn tool_running() -> Document {
    doc! {"activities": [{
        "id": "tool-1",
        "label": "private tool identifier",
        "status": "running",
        "started_at": BsonDateTime::now(),
    }]}
}

#[tokio::test]
async fn assistant_activity_route_is_private_metadata_only_and_human_only() {
    let f = Fixture::new("avatar_activity_route").await;
    let state = &f.state;
    let (agent, thread) = f.create().await;
    let member_thread = team::home_thread_for(&state.db, &state.encryption_keys, &f.member, &agent)
        .await
        .unwrap();
    set_turn(state, &thread, tool_running()).await;
    set_turn(
        state,
        &member_thread,
        doc! {"reply": {"message_id": "reply", "block_id": "block", "cursor": 1, "text": "private text"}},
    )
    .await;
    let (_, private) = crate::routes::build_router_with_state(state.clone());
    let app = private.with_state(state.clone());
    let owner_bearer = bearer(state, &f.admin);
    let member_bearer = bearer(state, &f.member);
    let outsider_bearer = bearer(state, &f.outsider);
    for (token, expected) in [
        (
            &owner_bearer,
            json!([{"agent_id": agent.id, "threads": [{"conversation_id": thread.id, "state": "working"}]}]),
        ),
        (
            &member_bearer,
            json!([{"agent_id": agent.id, "threads": [{"conversation_id": member_thread.id, "state": "drafting"}]}]),
        ),
        (&outsider_bearer, json!([])),
    ] {
        let (status, body) = snapshot(&app, ("authorization", token)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body, json!({"agents": expected}));
    }

    let Json(roster) = list_agents(
        State(state.clone()),
        test_auth_user(&f.member),
        Query(AgentsQuery::default()),
    )
    .await
    .unwrap();
    let summary = roster["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|summary| summary["id"] == agent.id)
        .unwrap();
    assert_eq!(summary["status"], "running");
    assert_eq!(
        summary["activity"],
        json!([{"conversation_id": member_thread.id, "state": "drafting"}])
    );

    let key = key_service::create_api_key(
        &state.db,
        &f.admin,
        "activity fixture",
        "proxy",
        None,
        None,
        Some(&[]),
        Some(&[]),
        Some(false),
        Some(false),
        Some(false),
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let (status, body) = snapshot(&app, ("x-api-key", &key.full_key)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let delegated = jwt::generate_delegated_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(&f.admin).unwrap(),
        "account:read",
        "activity-fixture",
        60,
        None,
    )
    .unwrap();
    let (status, body) = snapshot(&app, ("authorization", &format!("Bearer {delegated}"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // A retained personal thread does not retain use access when the specialist's
    // live organization audience is narrowed, even for a maintainer.
    state
        .db
        .collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &agent.id},
            doc! {"$set": {"org_user_ids": [&f.admin]}},
        )
        .await
        .unwrap();
    let (status, body) = snapshot(&app, ("authorization", &member_bearer)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({"agents": []}));
    let Json(roster) = list_agents(
        State(state.clone()),
        test_auth_user(&f.member),
        Query(AgentsQuery::default()),
    )
    .await
    .unwrap();
    let summary = roster["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == agent.id)
        .unwrap();
    assert_eq!(summary["can_use"], false);
    assert_eq!(summary["status"], "idle");
    assert_eq!(summary["activity"], json!([]));
    state
        .db
        .collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &agent.id},
            doc! {"$unset": {"org_user_ids": ""}},
        )
        .await
        .unwrap();

    state
        .db
        .collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
        .update_one(
            doc! {"org_user_id": &f.org, "member_user_id": &f.member},
            doc! {"$set": {"revoked_at": BsonDateTime::now()}},
        )
        .await
        .unwrap();
    let (status, body) = snapshot(&app, ("authorization", &member_bearer)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({"agents": []}));
    let leftover = state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"_id": &member_thread.id})
        .await
        .unwrap()
        .unwrap();
    assert!(live_turn(&leftover, Utc::now()).is_some());

    flags::set_platform_override(
        &state.db,
        flags::NYXAGENT_ENGINE_FLAG_KEY,
        &flags::FlagTarget::User(f.admin.clone()),
        false,
        &f.admin,
    )
    .await
    .unwrap();
    let (status, body) = snapshot(&app, ("authorization", &owner_bearer)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    f.readiness.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn assistant_activity_roster_and_details_filter_expired_collaboration_and_destroyed_threads()
{
    let f = Fixture::new("avatar_activity_filter").await;
    let state = &f.state;
    let owner = &f.admin;
    let (agent, thread) = team::create_specialist(
        &state.db,
        &state.encryption_keys,
        owner,
        f.request("activity-agent"),
    )
    .await
    .unwrap()
    .unwrap();
    let legacy =
        crate::services::assistant_authority_tests::new_orchestrator(state, owner, "Legacy").await;
    let nyxbot = team::ensure_nyxbot(&state.db, owner).await.unwrap();
    let rows = state.db.collection::<Document>(CONVERSATIONS);
    rows.update_one(doc! {"_id": &legacy.id}, doc! {"$unset": {"agent_id": 1}})
        .await
        .unwrap();
    set_turn(state, &legacy, doc! {"queued_at": BsonDateTime::now()}).await;
    set_turn(state, &thread, tool_running()).await;
    let original = rows
        .find_one(doc! {"_id": &thread.id})
        .await
        .unwrap()
        .unwrap();
    let mut expired = original.clone();
    expired.insert("_id", "nyxa-expired");
    expired
        .get_document_mut("active_turn")
        .unwrap()
        .insert("lease_expires_at", BsonDateTime::from_millis(0));
    let mut collaboration = original;
    collaboration.insert("_id", "nyxa-collaboration");
    collaboration.insert("collaboration", doc! {});
    rows.insert_many([expired, collaboration]).await.unwrap();

    let Json(value) = get_agent(
        State(state.clone()),
        test_auth_user(owner),
        Path(agent.id.clone()),
    )
    .await
    .unwrap();
    assert_eq!(
        value["agent"]["activity"],
        json!([{"conversation_id": thread.id, "state": "working"}])
    );
    assert_eq!(value["agent"]["status"], "running");
    let details = value["threads"].as_array().unwrap();
    assert_eq!(details.len(), 2);
    let live = details.iter().find(|row| row["id"] == thread.id).unwrap();
    assert_eq!(live["activity"], "working");
    assert_eq!(live["running"], true);
    let expired = details
        .iter()
        .find(|row| row["id"] == "nyxa-expired")
        .unwrap();
    assert_eq!(expired["activity"], Value::Null);
    assert_eq!(expired["running"], false);

    let Json(snapshot) = list_agent_activity(State(state.clone()), test_auth_user(owner))
        .await
        .unwrap();
    let snapshot = serde_json::to_value(snapshot).unwrap();
    let agents = snapshot["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 2);
    let legacy_entry = agents
        .iter()
        .find(|entry| entry["agent_id"] == nyxbot.id)
        .unwrap();
    assert_eq!(
        legacy_entry["threads"],
        json!([{"conversation_id": legacy.id, "state": "waiting"}])
    );

    set_turn(state, &thread, doc! {"stop_requested": true}).await;
    state
        .db
        .collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &agent.id},
            doc! {"$set": {"destroyed_at": BsonDateTime::now()}},
        )
        .await
        .unwrap();
    let Json(value) = get_agent(
        State(state.clone()),
        test_auth_user(owner),
        Path(agent.id.clone()),
    )
    .await
    .unwrap();
    assert_eq!(value["agent"]["activity"], json!([]));
    assert_eq!(value["agent"]["status"], "destroyed");
    let stopped_thread = value["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == thread.id)
        .unwrap();
    assert_eq!(stopped_thread["running"], true);
    assert_eq!(stopped_thread["activity"], Value::Null);
    let Json(snapshot) = list_agent_activity(State(state.clone()), test_auth_user(owner))
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(snapshot).unwrap(),
        json!({"agents": [{"agent_id": nyxbot.id, "threads": [{"conversation_id": legacy.id, "state": "waiting"}]}]}),
    );
    let identities = state
        .db
        .collection::<Document>(crate::models::assistant_agent::COLLECTION_NAME);
    identities
        .delete_one(doc! {"_id": &nyxbot.id})
        .await
        .unwrap();
    let Json(snapshot) = list_agent_activity(State(state.clone()), test_auth_user(owner))
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(snapshot).unwrap(),
        json!({"agents": []})
    );
    assert!(
        identities
            .find_one(doc! {"user_id": owner, "kind": "nyxbot"})
            .await
            .unwrap()
            .is_none()
    );
    f.readiness.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn assistant_activity_snapshot_and_roster_stay_within_the_existing_bound() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let reads = Arc::new(AtomicUsize::new(0));
    let aggregates = Arc::new(AtomicUsize::new(0));
    let find_count = reads.clone();
    let ack_count = aggregates.clone();
    let handler = EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            if event.command_name == "find" {
                find_count.fetch_add(1, Ordering::SeqCst);
            } else if event.command.get_str("aggregate").ok() == Some(ACKS) {
                ack_count.fetch_add(1, Ordering::SeqCst);
            }
        }
    });
    let db = crate::test_utils::connect_test_database_with_command_handler(
        "avatar_activity_bound",
        handler,
    )
    .await
    .unwrap();
    let f = Fixture::with_db(db).await;
    let state = &f.state;
    let (agent, thread) = team::create_specialist(
        &state.db,
        &state.encryption_keys,
        &f.admin,
        f.request("activity-bounded"),
    )
    .await
    .unwrap()
    .unwrap();
    set_turn(state, &thread, doc! {}).await;
    let rows = state.db.collection::<Document>(CONVERSATIONS);
    let template = rows
        .find_one(doc! {"_id": &thread.id})
        .await
        .unwrap()
        .unwrap();
    rows.insert_many((0..team::AGENT_ACTIVITY_LIMIT).map(|index| {
        let mut row = template.clone();
        row.insert("_id", format!("nyxa-activity-bound-{index}"));
        row
    }))
    .await
    .unwrap();
    let running = team::running_agents(&state.db, &f.admin).await.unwrap();
    assert_eq!(running[&agent.id].len() as i64, team::AGENT_ACTIVITY_LIMIT);
    let before = reads.load(Ordering::SeqCst);
    let ack_before = aggregates.load(Ordering::SeqCst);
    let snapshot = team::live_agent_activity(&state.db, &f.admin)
        .await
        .unwrap();
    assert_eq!(reads.load(Ordering::SeqCst) - before, 2);
    assert_eq!(aggregates.load(Ordering::SeqCst) - ack_before, 1);
    assert_eq!(snapshot[&agent.id].len() as i64, team::AGENT_ACTIVITY_LIMIT);
    f.readiness.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn assistant_activity_pending_cards_are_scoped_and_do_not_make_an_idle_agent_busy() {
    let f = Fixture::new("avatar_activity_approval").await;
    let state = &f.state;
    let owner = &f.admin;
    let (agent, thread) = team::create_specialist(
        &state.db,
        &state.encryption_keys,
        owner,
        f.request("activity-approval"),
    )
    .await
    .unwrap()
    .unwrap();
    let cards = state.db.collection::<Document>(ACKS);
    let deadline = BsonDateTime::from_chrono(Utc::now() + chrono::Duration::minutes(30));
    let card = |id, user: &str, status, expires_at| {
        doc! {
            "_id": id, "user_id": user, "conversation_id": &thread.id,
            "status": status, "expires_at": expires_at, "decider": "user",
        }
    };
    cards
        .insert_many([
            card("mine", owner, "pending", deadline),
            card("other", &f.outsider, "pending", deadline),
            card("expired", owner, "pending", BsonDateTime::from_millis(0)),
            card("decided", owner, "denied", deadline),
        ])
        .await
        .unwrap();

    let expected = json!([{"conversation_id": thread.id, "state": "approval"}]);
    let snapshot = team::live_agent_activity(&state.db, owner).await.unwrap();
    assert_eq!(
        serde_json::to_value(&snapshot[&agent.id]).unwrap(),
        expected
    );
    assert!(
        team::live_agent_activity(&state.db, &f.outsider)
            .await
            .unwrap()
            .is_empty()
    );
    let Json(roster) = list_agents(
        State(state.clone()),
        test_auth_user(owner),
        Query(AgentsQuery::default()),
    )
    .await
    .unwrap();
    let row = roster["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == agent.id)
        .unwrap();
    assert_eq!(row["activity"], expected);
    assert_eq!(row["status"], "idle");
    assert!(
        !team::running_agents(&state.db, owner)
            .await
            .unwrap()
            .contains_key(&agent.id)
    );

    let home = crate::services::assistant_authority_tests::new_orchestrator(
        state,
        owner,
        "Check specialists",
    )
    .await;
    let chat = acks::for_key(&state.db, owner, Some(&home.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let (listed, error) = execute_tool(state, &chat, "nyxid__list_subagents", &json!({})).await;
    assert!(!error, "{listed}");
    let row = listed["subagents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == agent.id)
        .unwrap();
    assert!(row.get("activity").is_none());
    assert_eq!(row["status"], "idle");
    let (waited, error) = execute_tool(
        state,
        &chat,
        "nyxid__wait_for_subagents",
        &json!({
            "subagents": [agent.id], "timeout_secs": 1,
        }),
    )
    .await;
    assert!(!error, "{waited}");
    assert_eq!(waited["running"], json!([]));
    assert_eq!(waited["settled"][0]["id"], agent.id);

    set_turn(state, &thread, tool_running()).await;
    assert_eq!(
        team::live_agent_activity(&state.db, owner).await.unwrap()[&agent.id][0].state,
        crate::services::assistant_activity::ActivityState::Approval
    );
    set_turn(state, &thread, doc! {"stop_requested": true}).await;
    assert_eq!(
        team::live_agent_activity(&state.db, owner).await.unwrap()[&agent.id][0].state,
        crate::services::assistant_activity::ActivityState::Stopping
    );
    state
        .db
        .collection::<Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &thread.id},
            doc! {"$set": {"active_turn": null}},
        )
        .await
        .unwrap();
    cards
        .update_one(doc! {"_id": "mine"}, doc! {"$set": {"status": "allowed"}})
        .await
        .unwrap();
    assert!(
        !team::live_agent_activity(&state.db, owner)
            .await
            .unwrap()
            .contains_key(&agent.id)
    );
    f.readiness.abort();
    state.db.drop().await.unwrap();
}
