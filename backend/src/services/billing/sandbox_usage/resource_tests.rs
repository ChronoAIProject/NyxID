use super::contract_tests::{decode, fixture};
use serde_json::{Value, json};

fn compute_without_home() -> Value {
    fixture("allocated-compute-only")
}

fn retained_home() -> Value {
    fixture("allocated-retained-home")
}

#[test]
fn compute_without_a_home_preserves_explicit_null_bindings() {
    let event = compute_without_home();
    let decoded = decode(&event).expect("compute does not require a home");
    let roundtrip: Value = serde_json::from_slice(decoded.canonical_bytes()).unwrap();
    assert_eq!(roundtrip, event);
    let validator = jsonschema::validator_for(&fixture("schema")).unwrap();
    assert!(validator.is_valid(&event));
    let mut transition = event;
    transition["facts"] = json!([]);
    assert!(validator.is_valid(&transition));
    assert!(decode(&transition).is_ok());
}

#[test]
fn retained_home_preserves_opaque_generation_without_runtime_binding() {
    let mut event = retained_home();
    for generation in [
        "opaque-home-generation",
        "0007",
        "9223372036854775808",
        "generation:alpha/7",
    ] {
        event["facts"][0]["resource"]["home_generation"] = json!(generation);
        let decoded = decode(&event).expect("home generation is an opaque identity");
        let roundtrip: Value = serde_json::from_slice(decoded.canonical_bytes()).unwrap();
        assert_eq!(roundtrip, event);
        let validator = jsonschema::validator_for(&fixture("schema")).unwrap();
        assert!(validator.is_valid(&event));
    }
}

#[test]
fn resource_fields_are_required_and_usage_requires_both_identity_components() {
    let validator = jsonschema::validator_for(&fixture("schema")).unwrap();
    for field in [
        "runtime_id",
        "runtime_generation",
        "home_id",
        "home_generation",
    ] {
        let mut event = fixture("allocated");
        event["facts"][0]["resource"][field] = Value::Null;
        assert_eq!(
            decode(&event).err(),
            Some("partial_resource_binding"),
            "{field}"
        );
        assert!(!validator.is_valid(&event), "partial {field}");
        event["facts"][0]["resource"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(decode(&event).is_err(), "omitted {field}");
        assert!(!validator.is_valid(&event), "omitted {field}");
    }
}

#[test]
fn inactive_bindings_still_require_complete_pairs() {
    let validator = jsonschema::validator_for(&fixture("schema")).unwrap();
    for (event, fields) in [
        (retained_home(), ["runtime_id", "runtime_generation"]),
        (compute_without_home(), ["home_id", "home_generation"]),
    ] {
        for field in fields {
            let mut partial = event.clone();
            partial["facts"][0]["resource"][field] = json!("7");
            assert_eq!(
                decode(&partial).err(),
                Some("partial_resource_binding"),
                "{field}"
            );
            assert!(!validator.is_valid(&partial), "{field}");
        }
    }
}

#[test]
fn usage_requires_its_binding_even_for_measured_residual_or_missing_evidence() {
    for name in ["allocated", "measured"] {
        for (id, generation) in [
            ("runtime_id", "runtime_generation"),
            ("home_id", "home_generation"),
        ] {
            let mut event = fixture(name);
            event["facts"][0]["resource"][id] = Value::Null;
            event["facts"][0]["resource"][generation] = Value::Null;
            if name == "measured" {
                event["facts"][0]["audit_time"]["compute_uptime_ms"] = json!("0");
                event["facts"][0]["audit_time"]["retained_home_ms"] = json!("0");
            }
            assert_eq!(
                decode(&event).err(),
                Some(if id == "runtime_id" {
                    "compute_binding_required"
                } else {
                    "home_binding_required"
                })
            );
        }
    }
    let mut event = fixture("measured");
    event["facts"][0] = fixture("measured-correction")["correction"]["replacement"].clone();
    event["facts"][0]["evidence_revision"] = json!("1");
    event["facts"][0]["resource"]["runtime_id"] = Value::Null;
    event["facts"][0]["resource"]["runtime_generation"] = Value::Null;
    event["facts"][0]["audit_time"]["compute_uptime_ms"] = json!("0");
    assert_eq!(decode(&event).err(), Some("compute_binding_required"));
}

#[test]
fn runtime_counter_and_opaque_home_text_keep_distinct_bounds() {
    let validator = jsonschema::validator_for(&fixture("schema")).unwrap();
    for (field, bad) in [
        ("runtime_generation", json!("0")),
        ("runtime_generation", json!("01")),
        ("runtime_generation", json!("9223372036854775808")),
        ("runtime_generation", json!("opaque")),
        ("home_generation", json!(1)),
        ("home_generation", json!("")),
        ("home_generation", json!(" padded")),
        ("home_generation", json!("control\ncharacter")),
        ("home_generation", json!("x".repeat(257))),
    ] {
        let mut event = fixture("allocated");
        event["facts"][0]["resource"][field] = bad;
        assert!(decode(&event).is_err(), "{field}");
        assert!(!validator.is_valid(&event), "{field}");
    }
}

#[test]
fn absent_inactive_binding_does_not_merge_distinct_active_resources() {
    for (mut event, active_id) in [
        (compute_without_home(), "runtime_id"),
        (retained_home(), "home_id"),
    ] {
        let mut second = event["facts"][0].clone();
        second["fact_id"] = json!("00000000-0000-4000-8000-000000000065");
        event["facts"].as_array_mut().unwrap().push(second);
        assert_eq!(decode(&event).err(), Some("overlapping_resource_facts"));
        event["facts"][1]["resource"][active_id] = json!("another-resource");
        assert!(decode(&event).is_ok());
    }
}

#[test]
fn corrections_pin_nullable_bindings_and_exact_opaque_generation() {
    for event in [compute_without_home(), retained_home()] {
        let before = decode(&event).unwrap();
        let mut correction = fixture("correction");
        correction["previous_event_digest"] = json!(before.digest().as_str());
        correction["correction"]["expected_predecessor_digest"] = json!(before.digest().as_str());
        correction["correction"]["replacement"] = event["facts"][0].clone();
        correction["correction"]["replacement"]["evidence_revision"] = json!("2");
        assert!(
            decode(&correction)
                .unwrap()
                .validate_correction_of(&before)
                .is_ok()
        );
        correction["correction"]["replacement"]["resource"]["home_id"] = json!("home-example");
        correction["correction"]["replacement"]["resource"]["home_generation"] =
            json!("changed-generation");
        assert!(
            decode(&correction)
                .unwrap()
                .validate_correction_of(&before)
                .is_err()
        );
    }
}
