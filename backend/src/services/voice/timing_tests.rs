use super::{progress, session, tests::setup, timing::*};
use crate::{
    models::{
        assistant_conversation::COLLECTION_NAME as THREADS,
        assistant_voice::{REQUESTS, RequestState},
        assistant_voice_session::COLLECTION_NAME as CALLS,
    },
    services::assistant_voice,
};
use mongodb::bson::{self, Document, doc};

#[test]
fn voice_phase_timing_announcement_never_claims_prior_or_interrupted_output() {
    use super::transcript::{Segment, Speaker};
    let mut output = Segment {
        id: uuid::Uuid::new_v4().to_string(),
        speaker: Speaker::Assistant,
        text: "Synthetic spoken update".into(),
        start_ms: 100,
        end_ms: 200,
        sealed: true,
        complete: true,
        interrupted: false,
        fragments: Vec::new(),
    };
    assert!(!announcement_matches(
        &output,
        "Synthetic spoken update",
        201
    ));
    assert!(!announcement_matches(
        &output,
        "Synthetic spoken update",
        150
    ));
    assert!(announcement_matches(&output, "Synthetic spoken update", 99));
    output.interrupted = true;
    assert!(!announcement_matches(
        &output,
        "Synthetic spoken update",
        99
    ));
    output.interrupted = false;
    output.speaker = Speaker::User;
    assert!(!announcement_matches(
        &output,
        "Synthetic spoken update",
        99
    ));
}

#[test]
fn voice_phase_timing_final_tool_snapshot_excludes_thinking_and_never_merges_positions() {
    use crate::models::assistant_conversation::TurnActivity;
    let start = chrono::Utc::now();
    let activity = |offset, digest: Option<String>| TurnActivity {
        id: uuid::Uuid::new_v4().to_string(),
        label: "private-label".into(),
        digest,
        retry_identity: None,
        status: "completed".into(),
        failure: None,
        memory_recovery: None,
        decision: None,
        machine: None,
        started_at: start + chrono::Duration::milliseconds(offset),
        ended_at: Some(start + chrono::Duration::milliseconds(offset + 10)),
    };
    let first = execution_snapshot(
        "turn",
        start,
        None,
        &[
            activity(10, Some("private-digest".into())),
            activity(20, None),
        ],
    );
    assert_eq!(first.tools.len(), 1);
    assert_eq!(first.tools[0].start_ms, 10);
    let final_snapshot = execution_snapshot(
        "turn",
        start,
        None,
        &[activity(300, Some("new-digest".into()))],
    );
    assert_eq!(final_snapshot.tools.len(), 1);
    assert_eq!(final_snapshot.tools[0].start_ms, 300);
    assert!(
        !serde_json::to_string(&final_snapshot)
            .unwrap()
            .contains("private")
    );
}

#[tokio::test]
async fn voice_phase_timing_legacy_marks_are_not_fabricated_phases() {
    let (state, _, call) = setup("voice_phase_legacy").await;
    let mut r = assistant_voice::enqueue(
        &state.db,
        &call.user_id,
        &call.conversation_id,
        &call.id,
        "source",
        "Synthetic task",
    )
    .await
    .unwrap();
    r.timing.extend([
        ("first_audio_ms".into(), 1000),
        ("tool_started_ms".into(), 20),
        ("first_tool_finished_ms".into(), 40),
        ("transcription_ms".into(), 10),
        ("secret-provider-identifier".into(), 50),
        ("result_ms".into(), -1),
    ]);
    let report = request_report(&r);
    assert!(
        report
            .measurements
            .iter()
            .any(|m| m.phase == Phase::PlaybackConfirmed
                && m.source == Source::ClientPlayback
                && m.kind == Kind::Offset)
    );
    assert!(!report.measurements.iter().any(|m| matches!(
        m.phase,
        Phase::FirstAudio | Phase::Tool | Phase::ToolStart | Phase::ToolEnd
    )));
    assert!(report.unavailable.contains(&"asr_processing".into()));
    assert!(report.unavailable.contains(&"synthesis_processing".into()));
    assert!(
        !serde_json::to_string(&report)
            .unwrap()
            .contains("secret-provider-identifier")
    );
    let snapshot = execution_snapshot(&r.turn_id, call.created_at, Some(&Default::default()), &[]);
    r.execution_timing = Some(snapshot);
    r.execution_timing.as_mut().unwrap().tools.push(
        crate::models::assistant_voice::VoiceToolTiming {
            start_ms: i64::MIN,
            end_ms: Some(i64::MAX),
        },
    );
    assert!(
        !request_report(&r)
            .measurements
            .iter()
            .any(|m| matches!(m.phase, Phase::Model | Phase::Tool))
    );
    r.execution_timing.as_mut().unwrap().turn_id = uuid::Uuid::new_v4().to_string();
    r.execution_timing.as_mut().unwrap().model_ms = Some(12345);
    assert!(
        !request_report(&r)
            .measurements
            .iter()
            .any(|m| m.phase == Phase::Model)
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn voice_phase_timing_observations_are_bounded_private_and_mixed_data_safe() {
    let (state, _, call) = setup("voice_phase_bounded").await;
    state.db.collection::<Document>(CALLS).update_one(doc!{"_id":&call.id}, doc!{"$set":{"phase_observations":[bson::Bson::Null, bson::Bson::Int32(2), bson::Bson::Document(doc!{"turn_id":"provider-id","kind":"input","raw_text":"private-canary"})]}}).await.unwrap();
    for _ in 0..35 {
        let mut report = Report::default();
        report.add(Phase::FirstAudio, Source::RelayAudio, Kind::Offset, 10);
        record(
            &state.db,
            &call,
            &uuid::Uuid::new_v4().to_string(),
            ObservationKind::VoiceResponse,
            report,
        )
        .await;
    }
    let saved = session::refresh(&state.db, &call).await.unwrap();
    assert_eq!(saved.phase_observations.len(), MAX_OBSERVATIONS);
    let evidence = observations(&saved);
    assert_eq!(evidence.len(), MAX_OBSERVATIONS);
    let wire = serde_json::to_string(&evidence).unwrap();
    for forbidden in [
        "private-canary",
        "provider-id",
        "raw_text",
        "source_id",
        "arguments",
        "transcript",
    ] {
        assert!(!wire.contains(forbidden));
    }
    let mut foreign = call.clone();
    foreign.user_id = uuid::Uuid::new_v4().to_string();
    record(
        &state.db,
        &foreign,
        &uuid::Uuid::new_v4().to_string(),
        ObservationKind::Input,
        Report::default(),
    )
    .await;
    assert_eq!(
        session::refresh(&state.db, &call)
            .await
            .unwrap()
            .phase_observations,
        saved.phase_observations
    );
    let mut mixed = saved;
    mixed.phase_observations.extend([
        bson::Bson::Null,
        bson::Bson::Document(doc! {"turn_id":"bad"}),
    ]);
    assert_eq!(observations(&mixed).len(), MAX_OBSERVATIONS - 2);
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn voice_phase_timing_refresh_correlates_thread_turn_call_and_owner() {
    let (state, thread, call) = setup("voice_phase_exact_join").await;
    let mut a = assistant_voice::enqueue(
        &state.db,
        &call.user_id,
        &thread.id,
        &call.id,
        "a",
        "Synthetic task",
    )
    .await
    .unwrap();
    a.state = RequestState::Claimed;
    let mut other = thread.clone();
    other.id = uuid::Uuid::new_v4().to_string();
    for (row, ms) in [(&mut other, 900)] {
        let active = row.active_turn.as_mut().unwrap();
        active.turn_id = a.turn_id.clone();
        active.timing.model_calls = 1;
        active.timing.model_ms = ms;
    }
    state
        .db
        .collection::<crate::models::assistant_conversation::AssistantConversation>(THREADS)
        .insert_one(&other)
        .await
        .unwrap();
    let mut active = thread.active_turn.clone().unwrap();
    active.turn_id = a.turn_id.clone();
    active.timing.model_calls = 1;
    active.timing.model_ms = 10;
    state
        .db
        .collection::<Document>(THREADS)
        .update_one(
            doc! {"_id":&thread.id},
            doc! {"$set":{"active_turn":bson::to_bson(&active).unwrap()}},
        )
        .await
        .unwrap();
    let mut b = a.clone();
    b.id = uuid::Uuid::new_v4().to_string();
    b.source_id = b.id.clone();
    b.task_conversation_id = Some(other.id.clone());
    state
        .db
        .collection::<Document>(REQUESTS)
        .update_one(doc! {"_id":&a.id}, doc! {"$set":{"state":"claimed"}})
        .await
        .unwrap();
    state
        .db
        .collection::<crate::models::assistant_voice::VoiceRequest>(REQUESTS)
        .insert_one(&b)
        .await
        .unwrap();
    let mut foreign = a.clone();
    foreign.session_id = uuid::Uuid::new_v4().to_string();
    foreign.execution_timing = None;
    let mut requests = [a, b, foreign];
    progress::refresh(&state.db, &call, &mut requests)
        .await
        .unwrap();
    assert_eq!(
        requests[0].execution_timing.as_ref().unwrap().model_ms,
        Some(10)
    );
    assert_eq!(
        requests[1].execution_timing.as_ref().unwrap().model_ms,
        Some(900)
    );
    assert!(requests[2].execution_timing.is_none());
    active.turn_id = "wrong-turn".into();
    assert_eq!(
        progress::project(&requests[0], Some(&active), chrono::Utc::now()).stage,
        "checking_status"
    );
    let status = progress::task_status(&state.db, &call).await.unwrap();
    assert!(
        status["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["phase_timing"]["unit"] == "ms")
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn voice_phase_timing_playback_requires_the_complete_audible_interval() {
    let (state, _, call) = setup("voice_phase_playback").await;
    let id = uuid::Uuid::new_v4().to_string();
    let mut pending = PendingPlayback::default();
    pending.push(
        id.clone(),
        ObservationKind::Output,
        100,
        200,
        Report::default(),
    );
    pending.confirm(&state.db, &call, 199, 0).await;
    assert!(
        session::refresh(&state.db, &call)
            .await
            .unwrap()
            .phase_observations
            .is_empty()
    );
    pending.confirm(&state.db, &call, 200, 150).await;
    assert!(
        session::refresh(&state.db, &call)
            .await
            .unwrap()
            .phase_observations
            .is_empty()
    );
    pending.push(id, ObservationKind::Output, 100, 200, Report::default());
    pending.confirm(&state.db, &call, 200, 100).await;
    let saved = session::refresh(&state.db, &call).await.unwrap();
    assert!(
        observations(&saved)[0]
            .phase_timing
            .measurements
            .iter()
            .any(|m| m.phase == Phase::PlaybackConfirmed)
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn voice_phase_timing_progress_does_not_repeat_an_intervening_stage() {
    let (state, _, call) = setup("voice_phase_progress_duplicates").await;
    let mut r = assistant_voice::enqueue(
        &state.db,
        &call.user_id,
        &call.conversation_id,
        &call.id,
        "source",
        "Synthetic task",
    )
    .await
    .unwrap();
    r.progress = Some(crate::models::assistant_voice::VoiceTaskProgress {
        stage: "working".into(),
        detail: "Checking issue progress".into(),
        last_completed_step: None,
        completed_steps: 0,
        updated_at: chrono::Utc::now(),
    });
    let mut coalescer = progress::Coalescer::default();
    assert!(coalescer.due(&r, 2000, 2000));
    r.progress.as_mut().unwrap().stage = "awaiting_approval".into();
    assert!(!coalescer.due(&r, 10000, 10000));
    r.progress.as_mut().unwrap().detail = "Waiting for approval".into();
    assert!(coalescer.due(&r, 10000, 10000));
    r.progress.as_mut().unwrap().stage = "working".into();
    r.progress.as_mut().unwrap().detail = "Checking issue progress".into();
    assert!(!coalescer.due(&r, 18000, 18000));
    state.db.drop().await.unwrap();
}
