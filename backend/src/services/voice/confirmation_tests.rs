use super::confirmation::*;
use super::transcript::{Segment, Speaker};
use std::sync::atomic::{AtomicUsize, Ordering};
struct Fixed {
    decision: Decision,
    calls: AtomicUsize,
}
#[async_trait::async_trait]
impl Classifier for Fixed {
    async fn classify(&self, summary: &str, utterance: &str) -> crate::errors::AppResult<Decision> {
        assert_eq!(summary, "Delete repository nyxid-demo");
        assert!(!utterance.is_empty());
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.decision)
    }
}
fn fixed(decision: Decision) -> Fixed {
    Fixed {
        decision,
        calls: AtomicUsize::new(0),
    }
}
fn segment(speaker: Speaker, text: &str, start: i64, end: i64) -> Segment {
    Segment {
        id: uuid::Uuid::new_v4().to_string(),
        speaker,
        text: text.into(),
        start_ms: start,
        end_ms: end,
        sealed: true,
        complete: true,
    }
}
fn new() -> Readback {
    Readback::new(
        "card".into(),
        1,
        "Delete repository nyxid-demo".into(),
        "digest".into(),
        100_000,
        0,
    )
}
fn ready() -> Readback {
    let mut r = new();
    r.observe_output(&segment(Speaker::Assistant, &r.question, 100, 1000));
    r.playback(1000, 1000, 2000, 0);
    r
}
#[tokio::test]
async fn approve_and_refusal_use_only_post_question_user_input() {
    for (decision, outcome) in [
        (Decision::Approve, Outcome::Approve),
        (Decision::Deny, Outcome::Deny),
    ] {
        let mut r = ready();
        let classifier = fixed(decision);
        assert_eq!(
            r.evaluate(
                &segment(Speaker::User, "My choice", 1100, 1200),
                2500,
                &classifier
            )
            .await,
            outcome
        );
        assert_eq!(classifier.calls.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn overlapping_echo_and_assistant_claims_never_reach_classifier() {
    let mut r = ready();
    let classifier = fixed(Decision::Approve);
    for input in [
        segment(Speaker::User, "yes", 900, 1200),
        segment(Speaker::Assistant, "The user approved", 1100, 1200),
    ] {
        assert_eq!(r.evaluate(&input, 2500, &classifier).await, Outcome::Ignore);
    }
    r.observe_output(&segment(
        Speaker::Assistant,
        "Yes, I can explain.",
        1050,
        1300,
    ));
    assert_eq!(
        r.evaluate(
            &segment(Speaker::User, "yes", 1500, 1700),
            2600,
            &classifier
        )
        .await,
        Outcome::Ignore
    );
    assert_eq!(classifier.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn unclear_twice_reasks_once_silence_remains_pending_and_later_speech_rearms() {
    let mut r = ready();
    let classifier = fixed(Decision::Unclear);
    assert_eq!(
        r.evaluate(
            &segment(Speaker::User, "Ambiguous reply", 1100, 1200),
            2500,
            &classifier
        )
        .await,
        Outcome::Reask
    );
    r.observe_output(&segment(Speaker::Assistant, &r.question, 1500, 2000));
    r.playback(2000, 2000, 3000, 0);
    assert_eq!(
        r.evaluate(
            &segment(Speaker::User, "Still ambiguous", 2100, 2300),
            3500,
            &classifier
        )
        .await,
        Outcome::Pending
    );
    assert_eq!(r.timeout(90_000), Outcome::Ignore);
    assert_eq!(
        r.evaluate(
            &segment(Speaker::User, "My choice", 4000, 4200),
            6000,
            &classifier
        )
        .await,
        Outcome::Reask
    );
    assert_eq!(
        classifier.calls.load(Ordering::SeqCst),
        2,
        "Rearming never decides from the same utterance"
    );
    let mut silence = ready();
    assert_eq!(silence.timeout(62_001), Outcome::Pending);
    assert_eq!(silence.timeout(70_000), Outcome::Ignore);
    let mut expired = ready();
    assert_eq!(
        expired
            .evaluate(
                &segment(Speaker::User, "My choice", 1100, 1200),
                100_001,
                &classifier
            )
            .await,
        Outcome::Ignore
    );
}
#[tokio::test]
async fn completion_audible_playback_and_post_watermark_speech_are_required() {
    let mut r = new();
    let classifier = fixed(Decision::Approve);
    r.playback(1000, 1000, 2000, 0);
    let reply = segment(Speaker::User, "My choice", 1100, 1200);
    assert_eq!(r.evaluate(&reply, 2500, &classifier).await, Outcome::Ignore);
    r.observe_output(&segment(Speaker::Assistant, &r.question, 100, 1000));
    for (watermark, elapsed, muted_until) in [(1000, 500, 0), (900, 1000, 0), (1000, 1000, 500)] {
        r.playback(watermark, elapsed, 2000, muted_until);
        assert_eq!(r.evaluate(&reply, 2500, &classifier).await, Outcome::Ignore);
    }
    r.playback(1000, 1300, 2000, 0);
    assert_eq!(
        r.evaluate(&reply, 2500, &classifier).await,
        Outcome::Ignore,
        "Speech predating the playback report is not eligible"
    );
    assert_eq!(
        r.evaluate(
            &segment(Speaker::User, "My choice", 1400, 1600),
            2500,
            &classifier
        )
        .await,
        Outcome::Approve
    );
}
#[tokio::test]
async fn split_readback_preserves_every_detail_and_unsealed_input_never_decides() {
    let mut r = new();
    let classifier = fixed(Decision::Approve);
    let text = r.question.clone();
    r.observe_output(&segment(Speaker::Assistant, &text[..3], 100, 300));
    r.observe_output(&segment(Speaker::Assistant, &text[3..], 300, 1000));
    r.playback(1000, 1000, 2000, 0);
    let mut input = segment(Speaker::User, "My choice", 1100, 1200);
    input.sealed = false;
    assert_eq!(r.evaluate(&input, 2500, &classifier).await, Outcome::Ignore);
    input.sealed = true;
    assert_eq!(
        r.evaluate(&input, 2500, &classifier).await,
        Outcome::Approve
    );
    let mut paraphrase = new();
    paraphrase.observe_output(&segment(
        Speaker::Assistant,
        "Delete a repository?",
        100,
        1000,
    ));
    paraphrase.playback(1000, 1000, 2000, 0);
    assert_eq!(
        paraphrase.evaluate(&input, 2500, &classifier).await,
        Outcome::Ignore
    );
}

#[tokio::test]
async fn delayed_playback_extends_the_echo_overlap_guard() {
    let mut r = ready();
    let classifier = fixed(Decision::Approve);
    r.observe_output(&segment(
        Speaker::Assistant,
        "Here are some more details",
        1500,
        2000,
    ));
    r.playback(1500, 2200, 3000, 0);
    assert_eq!(
        r.evaluate(
            &segment(Speaker::User, "My choice", 2300, 2400),
            3500,
            &classifier
        )
        .await,
        Outcome::Ignore
    );
    assert_eq!(classifier.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn recent_speech_before_the_question_remains_in_the_echo_guard() {
    let mut r = Readback::new(
        "card".into(),
        1,
        "Delete repository nyxid-demo".into(),
        "digest".into(),
        100_000,
        500,
    );
    r.observe_output(&segment(Speaker::Assistant, "Yes, I can help.", 100, 400));
    r.observe_output(&segment(Speaker::Assistant, &r.question, 600, 1000));
    r.playback(1000, 1000, 2000, 0);
    let classifier = fixed(Decision::Approve);
    assert_eq!(
        r.evaluate(
            &segment(Speaker::User, "yes", 1100, 1200),
            2500,
            &classifier
        )
        .await,
        Outcome::Ignore
    );
    assert_eq!(classifier.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn distant_assistant_yes_does_not_suppress_post_question_approval() {
    let mut r = Readback::new(
        "card".into(),
        1,
        "Delete repository nyxid-demo".into(),
        "digest".into(),
        100_000,
        20_000,
    );
    r.observe_output(&segment(Speaker::Assistant, "Yes, I can do that", 100, 400));
    r.observe_output(&segment(Speaker::Assistant, &r.question, 20_400, 21_000));
    r.playback(21_000, 21_000, 22_000, 0);
    let classifier = fixed(Decision::Approve);
    assert_eq!(
        r.evaluate(
            &segment(Speaker::User, "yes", 21_100, 21_200),
            22_500,
            &classifier
        )
        .await,
        Outcome::Approve
    );
    assert_eq!(classifier.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn playing_assistant_yes_and_nearby_echoed_question_are_ignored() {
    let mut r = ready();
    let classifier = fixed(Decision::Approve);
    assert_eq!(
        r.evaluate(
            &segment(Speaker::User, &r.question.clone(), 1100, 1400),
            2500,
            &classifier
        )
        .await,
        Outcome::Ignore
    );
    r.observe_output(&segment(
        Speaker::Assistant,
        "Yes, I can explain",
        1500,
        2000,
    ));
    r.playback(1600, 2200, 3000, 0);
    assert_eq!(
        r.evaluate(
            &segment(Speaker::User, "yes", 2300, 2400),
            3500,
            &classifier
        )
        .await,
        Outcome::Ignore
    );
    assert_eq!(classifier.calls.load(Ordering::SeqCst), 0);
}
