use super::contract_tests::{decode, fixture};
use serde_json::json;

#[test]
fn allocated_and_measured_events_advance_their_own_stream() {
    for chain in [
        &["stream-open", "allocated", "correction", "stream-seal"][..],
        &["measured-stream-open", "measured", "measured-correction"][..],
    ] {
        for pair in chain.windows(2) {
            let head = decode(&fixture(pair[0])).unwrap();
            let next = decode(&fixture(pair[1])).unwrap();
            assert_eq!(next.validate_successor_of(&head), Ok(()), "{pair:?}");
        }
    }
}

#[test]
fn sequence_and_digest_must_follow_the_durable_head() {
    let opening = decode(&fixture("stream-open")).unwrap();
    let skipped = decode(&fixture("correction")).unwrap();
    assert_eq!(
        skipped.validate_successor_of(&opening),
        Err("stream_chain_discontinuity")
    );
    let mut wrong_digest = fixture("allocated");
    wrong_digest["previous_event_digest"] = json!("b".repeat(64));
    assert_eq!(
        decode(&wrong_digest)
            .unwrap()
            .validate_successor_of(&opening),
        Err("stream_chain_discontinuity")
    );
}

#[test]
fn following_the_head_cannot_change_owner_or_append_after_seal() {
    let opening = decode(&fixture("stream-open")).unwrap();
    let mut changed_owner = fixture("allocated");
    changed_owner["owner_ref"]["subject"] = json!("another-fixture-owner");
    assert_eq!(
        decode(&changed_owner)
            .unwrap()
            .validate_successor_of(&opening),
        Err("stream_binding_changed")
    );
    let seal = decode(&fixture("stream-seal")).unwrap();
    let mut after_seal = fixture("allocated");
    after_seal["sequence"] = json!("5");
    after_seal["previous_event_digest"] = json!(seal.digest().as_str());
    assert_eq!(
        decode(&after_seal).unwrap().validate_successor_of(&seal),
        Err("stream_sealed")
    );
}
