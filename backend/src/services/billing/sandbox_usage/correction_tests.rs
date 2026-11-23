use super::{
    contract::*,
    contract_tests::{clock, decode, fixture},
};
use serde_json::{Value, json};

#[test]
fn corrections_pin_predecessor_and_allow_full_evidence_replacement() {
    for (old, new) in [
        ("allocated", "correction"),
        ("measured", "measured-correction"),
    ] {
        let before = decode(&fixture(old)).unwrap();
        let after = decode(&fixture(new)).unwrap();
        assert!(after.validate_correction_of(&before).is_ok());
    }
    let before = decode(&fixture("correction")).unwrap();
    let mut next = fixture("correction");
    next["sequence"] = json!("4");
    next["event_id"] = json!("00000000-0000-4000-8000-00000000000f");
    next["previous_event_digest"] = json!(before.digest().as_str());
    next["correction"]["expected_predecessor_event_id"] = json!(before.header().event_id.as_str());
    next["correction"]["expected_predecessor_digest"] = json!(before.digest().as_str());
    next["correction"]["expected_evidence_revision"] = json!("2");
    next["correction"]["replacement"]["evidence_revision"] = json!("3");
    assert!(
        decode(&next)
            .unwrap()
            .validate_correction_of(&before)
            .is_ok()
    );
    assert!(
        decode(&next)
            .unwrap()
            .validate_correction_of(&decode(&fixture("allocated")).unwrap())
            .is_err()
    );
}

#[test]
fn correction_cannot_retarget_owner_resource_basis_period_or_predecessor() {
    let before = decode(&fixture("allocated")).unwrap();
    for (path, v) in [
        ("/owner_ref/subject", json!("other-owner")),
        ("/owner_ref/issuer", json!("other-issuer")),
        ("/owner_ref/kind", json!("organization")),
        ("/producer_namespace", json!("other-producer")),
        ("/sandbox_id", json!("other-sandbox")),
        ("/stream_id", json!("00000000-0000-4000-8000-000000000099")),
        (
            "/source_epoch",
            json!("00000000-0000-4000-8000-000000000099"),
        ),
        ("/sequence", json!("2")),
        (
            "/correction/expected_predecessor_event_id",
            json!("00000000-0000-4000-8000-000000000099"),
        ),
        (
            "/correction/expected_predecessor_digest",
            json!("b".repeat(64)),
        ),
        (
            "/correction/replacement/resource/runtime_id",
            json!("other-runtime"),
        ),
        (
            "/correction/replacement/resource/runtime_generation",
            json!("8"),
        ),
        (
            "/correction/replacement/resource/home_id",
            json!("other-home"),
        ),
        (
            "/correction/replacement/resource/home_generation",
            json!("3"),
        ),
    ] {
        let mut value = fixture("correction");
        *value.pointer_mut(path).unwrap() = v;
        assert!(
            decode(&value)
                .unwrap()
                .validate_correction_of(&before)
                .is_err(),
            "accepted mutation at {path}"
        );
    }
    let mut period = fixture("correction");
    for field in ["bucket_start_ms", "start_ms", "end_ms"] {
        let time = period["correction"]["replacement"][field]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap();
        period["correction"]["replacement"][field] = json!((time - 60000).to_string());
    }
    assert!(
        decode(&period)
            .unwrap()
            .validate_correction_of(&before)
            .is_err()
    );
    assert!(
        decode(&fixture("measured-correction"))
            .unwrap()
            .validate_correction_of(&before)
            .is_err()
    );
    assert!(
        decode(&fixture("allocated"))
            .unwrap()
            .validate_correction_of(&before)
            .is_err()
    );
}

#[test]
fn malformed_correction_and_revision_overflow_refuse() {
    for (path, value) in [
        ("/correction/reason", json!("refund")),
        (
            "/correction/expected_evidence_revision",
            json!("9223372036854775807"),
        ),
        ("/correction/replacement/evidence_revision", json!("1")),
        (
            "/correction/fact_id",
            json!("00000000-0000-4000-8000-000000000099"),
        ),
        ("/correction/replacement", Value::Null),
        (
            "/correction/expected_predecessor_event_id",
            fixture("correction")["event_id"].clone(),
        ),
    ] {
        let mut changed = fixture("correction");
        *changed.pointer_mut(path).unwrap() = value;
        assert!(decode(&changed).is_err());
    }
    let mut changed = fixture("correction");
    changed["correction"]["delta"] = json!("-1");
    assert!(decode(&changed).is_err());
}

#[test]
fn receipt_proves_only_exact_custody_and_cannot_mean_charged() {
    let event = decode(&fixture("allocated")).unwrap();
    let receipt =
        decode_receipt(&serde_json::to_vec(&fixture("receipt")).unwrap(), clock()).unwrap();
    assert!(receipt.matches(&event));
    for (field, value) in [
        ("event_id", json!("00000000-0000-4000-8000-000000000099")),
        ("stream_id", json!("00000000-0000-4000-8000-000000000099")),
        ("sequence", json!("3")),
        ("payload_digest", json!("0".repeat(64))),
        ("accepted_at_ms", json!("1795132800000")),
    ] {
        let mut changed = fixture("receipt");
        changed[field] = value;
        assert!(
            !decode_receipt(&serde_json::to_vec(&changed).unwrap(), clock())
                .unwrap()
                .matches(&event)
        );
    }
    for (field, value) in [
        ("custody", json!("charged")),
        ("charged", json!(true)),
        ("financial_status", json!("settled")),
    ] {
        let mut changed = fixture("receipt");
        changed[field] = value;
        assert!(decode_receipt(&serde_json::to_vec(&changed).unwrap(), clock()).is_err());
    }
    let raw = serde_json::to_string(&fixture("receipt"))
        .unwrap()
        .replacen(
            "\"custody\":\"persisted\"",
            "\"custody\":\"persisted\",\"custody\":\"persisted\"",
            1,
        );
    assert!(decode_receipt(raw.as_bytes(), clock()).is_err());
}
