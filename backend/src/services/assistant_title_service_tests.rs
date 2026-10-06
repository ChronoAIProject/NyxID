use super::*;
use crate::{
    models::user::{COLLECTION_NAME as USERS, UserType},
    services::{assistant_nyxagent as engine, assistant_team_service as team},
    test_utils::{connect_transaction_test_database, test_app_state, test_user},
};

async fn fixture() -> (crate::AppState, String, AssistantConversation) {
    let db = connect_transaction_test_database("assistant_titles").await;
    let owner = uuid::Uuid::new_v4().to_string();
    db.collection(USERS)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let state = test_app_state(db);
    let agent = Box::pin(team::ensure_nyxbot(&state.db, &owner))
        .await
        .unwrap();
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let row = Box::pin(team::create_thread_for(
        &state.db,
        &state.encryption_keys,
        &owner,
        &agent,
        "New chat",
        &mut session,
    ))
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    (state, owner, row)
}

#[test]
fn provisional_first_line_whitespace_unicode_and_word_boundary() {
    assert_eq!(
        provisional(" \n  Plan  my\tweek\nIgnore this line"),
        "Plan my week"
    );
    assert_eq!(provisional(""), "New chat");
    let long = "Research the architectural implications of moving all authentication services";
    assert_eq!(
        provisional(long),
        "Research the architectural implications of moving all"
    );
    assert_eq!(provisional(&"界".repeat(80)), "界".repeat(60));
}

#[test]
fn generated_is_bounded_untrusted_plain_text() {
    assert_eq!(
        generated("\"**Plan\nour\tweek**\""),
        Some("Plan our week".into())
    );
    assert_eq!(generated("计划下周的工作。"), Some("计划下周的工作".into()));
    assert_eq!(
        generated("\u{0}Build\u{202e} a plan!"),
        Some("Build a plan".into())
    );
    assert_eq!(generated("..."), None);
    assert!(generated(&"word ".repeat(100)).unwrap().chars().count() <= 60);
}

#[tokio::test]
async fn generated_applies_once_and_rename_wins_over_late_generation() {
    let (state, owner, row) = fixture().await;
    assert!(
        Box::pin(apply(&state.db, &owner, &row.id, "A helpful title"))
            .await
            .unwrap()
    );
    assert!(
        !Box::pin(apply(&state.db, &owner, &row.id, "A second generation"))
            .await
            .unwrap()
    );
    let current = engine::get(&state.db, &owner, &row.id).await.unwrap();
    assert_eq!(current.title_source, TitleSource::Generated);
    assert_eq!(current.title, "A helpful title");
    // Reset the fixture to model a generation whose upstream response is still pending.
    state
        .db
        .collection::<mongodb::bson::Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"title_source": "provisional"}},
        )
        .await
        .unwrap();
    Box::pin(engine::rename(&state.db, &owner, &row.id, "My title"))
        .await
        .unwrap();
    assert!(
        !Box::pin(apply(&state.db, &owner, &row.id, "Late generated title"))
            .await
            .unwrap()
    );
    let current = engine::get(&state.db, &owner, &row.id).await.unwrap();
    assert_eq!(current.title_source, TitleSource::User);
    assert_eq!(current.title, "My title");
    assert!(!format!("{current:?}").contains("My title"));
}

#[tokio::test]
async fn legacy_rows_and_guest_threads_are_never_generated() {
    let (state, owner, row) = fixture().await;
    state
        .db
        .collection::<mongodb::bson::Document>(COLLECTION_NAME)
        .update_one(doc! {"_id": &row.id}, doc! {"$unset": {"title_source": ""}})
        .await
        .unwrap();
    let legacy = engine::get(&state.db, &owner, &row.id).await.unwrap();
    assert_eq!(legacy.title_source, TitleSource::User);
    assert!(
        !Box::pin(apply(&state.db, &owner, &row.id, "Generated"))
            .await
            .unwrap()
    );
    assert!(
        Box::pin(first_exchange(&state.db, &owner, &row.id))
            .await
            .unwrap()
            .is_none()
    );
    let mut guest = row.clone();
    guest.guest_turn = true;
    assert!(!eligible(&guest));
    guest.guest_turn = false;
    guest.group_id = Some("group".into());
    assert!(!eligible(&guest));
    assert!(
        Box::pin(apply(&state.db, "another-owner", &row.id, "Generated"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn rename_during_active_turn_survives_settlement_and_provisional_uses_first_message() {
    let (state, owner, row) = fixture().await;
    let request = serde_json::from_value::<engine::TurnRequest>(
        serde_json::json!({"conversation_id":row.id,"text":"  Plan  a trip\nwith details"}),
    )
    .unwrap();
    let start: engine::TurnStart = (&request).into();
    let active = Box::pin(engine::begin_turn(
        &state.db,
        &owner,
        &start,
        &state.encryption_keys,
    ))
    .await
    .unwrap();
    assert_eq!(active.title, "Plan a trip");
    assert_eq!(active.title_source, TitleSource::Provisional);
    let renamed = Box::pin(engine::rename(&state.db, &owner, &row.id, "Owner choice"))
        .await
        .unwrap();
    assert!(renamed.active_turn.is_some());
    let result = engine::TurnResult {
        text: "Here is a plan".into(),
        session_id: None,
        response_id: None,
        error: None,
    };
    Box::pin(engine::finish_turn(
        &state.db,
        &active,
        &active.credential_api_key_id,
        &uuid::Uuid::new_v4().to_string(),
        &result,
    ))
    .await
    .unwrap();
    assert_eq!(
        engine::get(&state.db, &owner, &row.id).await.unwrap().title,
        "Owner choice"
    );
    assert!(
        Box::pin(first_exchange(&state.db, &owner, &row.id))
            .await
            .unwrap()
            .is_none()
    );
}
