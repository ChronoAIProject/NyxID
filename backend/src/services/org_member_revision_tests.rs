use super::*;
use crate::services::{
    assistant_agent_learning as learning, assistant_voice as voice, trigger_schedule as schedules,
    trigger_service, voice::session,
};

async fn enable(f: &Fixture, keys: &[&str]) {
    for key in keys {
        flags::set_platform_override(&f.state.db, key, &flags::FlagTarget::Global, true, &f.admin)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn eligibility_revision_checks_personal_cards_at_decision_and_consumption() {
    let f = Fixture::new("member_target_card").await;
    let db = &f.state.db;
    let (agent, _) = f.create().await;
    let bot = team::ensure_nyxbot(db, &f.member).await.unwrap();
    let thread = team::home_thread(db, &f.state.encryption_keys, &bot)
        .await
        .unwrap();
    let chat = acks::for_key(db, &f.member, Some(&thread.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let tool = "nyxid__update_subagent";
    let args = serde_json::json!({"subagent":agent.id});
    let request = || {
        acks::request(
            db,
            &chat,
            acks::Request {
                kind: "action",
                service: None,
                tool: Some(tool),
                arguments: Some(&args),
                summary: "Update specialist",
                platform: false,
            },
        )
    };
    let allowed = request().await.unwrap();
    acks::decide(db, &f.member, &thread.id, &allowed.id, true)
        .await
        .unwrap();
    let pending = request().await.unwrap();
    f.repromote_member().await;
    assert!(
        acks::decide(db, &f.member, &thread.id, &pending.id, true)
            .await
            .is_err()
    );
    assert!(
        !acks::consume_action(db, &chat, &allowed.id, tool, &args)
            .await
            .unwrap()
    );
    let fresh = request().await.unwrap();
    assert_ne!(fresh.id, pending.id);
    acks::decide(db, &f.member, &thread.id, &fresh.id, true)
        .await
        .unwrap();
    assert!(
        acks::consume_action(db, &chat, &fresh.id, tool, &args)
            .await
            .unwrap()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn eligibility_revision_checks_trigger_selection_occurrence_and_thread() {
    use crate::models::{
        trigger::{TriggerDelivery, TriggerTokenLocation, TriggerVerification},
        trigger_schedule::{DeliverTo, ThreadPolicy},
    };
    let f = Fixture::new("member_trigger").await;
    let db = &f.state.db;
    let (agent, _) = f.create().await;
    enable(&f, &[flags::NYXAGENT_ENGINE_FLAG_KEY]).await;
    let create = || {
        trigger_service::create(
            db,
            &f.state.encryption_keys,
            trigger_service::CreateInput {
                source: Default::default(),
                setup_watch_id: None,
                schedule: None,
                overlap: Default::default(),
                user_id: f.member.clone(),
                label: "Scheduled task".into(),
                user_service_id: None,
                verification: TriggerVerification::Token {
                    location: TriggerTokenLocation::Bearer,
                },
                delivery: TriggerDelivery::Assistant {
                    agent_id: agent.id.clone(),
                    instruction: "Prepare report".into(),
                    thread_policy: Some(ThreadPolicy::Home),
                    confirmation_policy: Default::default(),
                    deliver_to: DeliverTo::Thread,
                },
            },
        )
    };
    let original = create().await.unwrap().trigger;
    let pending = schedules::enqueue_now(db, &original).await.unwrap();
    f.repromote_member().await;
    let stale = schedules::enqueue_now(db, &original).await.unwrap();
    assert_eq!(stale.reason.as_deref(), Some("target_unavailable"));
    assert!(
        schedules::run_thread(db, &f.state.encryption_keys, &pending, &original)
            .await
            .is_err()
    );
    assert_eq!(
        db.collection::<bson::Document>("assistant_conversations")
            .count_documents(doc! {"user_id":&f.member,"agent_id":&agent.id})
            .await
            .unwrap(),
        0
    );
    let fresh = create().await.unwrap().trigger;
    let run = schedules::enqueue_now(db, &fresh).await.unwrap();
    let row = schedules::run_thread(db, &f.state.encryption_keys, &run, &fresh)
        .await
        .unwrap();
    assert_eq!(row.member_use.eligibility_revision, 1);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn eligibility_revision_checks_voice_call_and_queued_work() {
    use crate::models::assistant_voice::{VoiceKeySource, VoicePreferences};
    let f = Fixture::new("member_voice").await;
    let db = &f.state.db;
    let (agent, _) = f.create().await;
    enable(
        &f,
        &[
            flags::NYXAGENT_ENGINE_FLAG_KEY,
            flags::ASSISTANT_VOICE_FLAG_KEY,
        ],
    )
    .await;
    let row = team::home_thread_for(db, &f.state.encryption_keys, &f.member, &agent)
        .await
        .unwrap();
    let preferences = VoicePreferences {
        service_id: Uuid::new_v4().to_string(),
        connection_id: None,
        key_source: VoiceKeySource::Platform,
        model: "voice".into(),
        voice: None,
        input_mode: Default::default(),
        language: None,
        notify_on_completion: false,
    };
    let call = session::admit(
        db,
        &f.member,
        &row.id,
        &Uuid::new_v4().to_string(),
        preferences,
        "fixture".into(),
        "test",
    )
    .await
    .unwrap();
    let request = voice::enqueue(
        db,
        &f.member,
        &row.id,
        &call.id,
        "original",
        "Prepare a report",
    )
    .await
    .unwrap();
    f.repromote_member().await;
    assert!(session::refresh(db, &call).await.is_err());
    assert!(
        voice::ensure_task_conversation(db, &f.state.encryption_keys, &request)
            .await
            .is_err()
    );
    session::close(db, &call, "ended", true).await.unwrap();
    let call = session::admit(
        db,
        &f.member,
        &row.id,
        &Uuid::new_v4().to_string(),
        call.preferences.clone(),
        "fixture".into(),
        "test",
    )
    .await
    .unwrap();
    session::refresh(db, &call).await.unwrap();
    let fresh = voice::enqueue(
        db,
        &f.member,
        &row.id,
        &call.id,
        "fresh",
        "Prepare a report",
    )
    .await
    .unwrap();
    assert_eq!(fresh.eligibility_revision, 1);
    assert!(
        voice::ensure_task_conversation(db, &f.state.encryption_keys, &fresh)
            .await
            .unwrap()
            .task_conversation_id
            .is_some()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn eligibility_revision_checks_learning_consent_after_membership_loss() {
    let f = Fixture::new("member_learning").await;
    let db = &f.state.db;
    let (agent, _) = f.create().await;
    enable(&f, &[flags::NYXAGENT_ENGINE_FLAG_KEY, learning::FLAG_KEY]).await;
    let config = learning::configure(db, &f.admin, &agent.id, true, 2)
        .await
        .unwrap();
    learning::set_member_opt_in(db, &f.member, &agent.id, true)
        .await
        .unwrap();
    assert_eq!(
        learning::enrollment_epoch(db, &f.member, &agent, None)
            .await
            .unwrap(),
        Some(config.learning_epoch)
    );
    f.repromote_member().await;
    assert_eq!(
        learning::enrollment_epoch(db, &f.member, &agent, None)
            .await
            .unwrap(),
        None
    );
    let fresh = learning::set_member_opt_in(db, &f.member, &agent.id, true)
        .await
        .unwrap();
    assert_eq!(fresh.eligibility_revision, 1);
    assert_eq!(
        learning::enrollment_epoch(db, &f.member, &agent, None)
            .await
            .unwrap(),
        Some(config.learning_epoch)
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn eligibility_revision_survives_person_reactivation() {
    let f = Fixture::new("member_reactivation").await;
    let db = &f.state.db;
    let (agent, _) = f.create().await;
    let row = team::home_thread_for(db, &f.state.encryption_keys, &f.member, &agent)
        .await
        .unwrap();
    let original = f.key(&row).await;
    for active in [false, false, true] {
        crate::services::admin_user_service::set_user_active(db, &f.admin, &f.member, active)
            .await
            .unwrap();
    }
    let membership = crate::services::org_service::get_active_membership(db, &f.org, &f.member)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(membership.eligibility_revision, 1);
    assert!(
        org_agents::resolve_credential_access(db, &original)
            .await
            .is_err()
    );
    let fresh = engine::begin_turn(
        db,
        &f.member,
        &engine::TurnRequest {
            conversation_id: Some(row.id),
            acknowledgement_id: None,
            retry_of: None,
            attachment_ids: vec![],
            agent_id: None,
            text: "Fresh task".into(),
            model: None,
            access_mode: None,
        },
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    assert_eq!(fresh.member_use.eligibility_revision, 1);
    db.drop().await.unwrap();
}
