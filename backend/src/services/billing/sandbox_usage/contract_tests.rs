use super::contract::*;
use serde_json::{Value, json};

pub(super) const NOW: u64 = 1_795_132_900_000;
pub(super) fn clock() -> Clock {
    Clock {
        now_ms: NOW,
        allowed_clock_skew_ms: 0,
    }
}
pub(super) fn fixture(name: &str) -> Value {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cma_usage_v1");
    serde_json::from_slice(&std::fs::read(root.join(format!("{name}.json"))).unwrap()).unwrap()
}
pub(super) fn decode(value: &Value) -> ContractResult<Event> {
    decode_event(&serde_json::to_vec(value).unwrap(), clock())
}
fn rejects(value: &Value) {
    assert!(decode(value).is_err(), "invalid synthetic event accepted");
}

#[test]
fn strict_discriminants_fields_and_variant_payloads() {
    for (key, value) in [
        ("schema_version", json!(2)),
        ("schema_version", json!("1")),
        ("kind", json!("future")),
        ("basis", json!("average")),
        ("metric_definition_version", json!("allocated-v2")),
        ("algorithm_version", json!("lifecycle-minute-split-v2")),
        ("algorithm_version", json!("binding-integral-half-even-v1")),
        ("wallet_amount", json!("1")),
        ("price", json!("1")),
        ("funded_window_id", json!("synthetic")),
        ("digest", json!("a".repeat(64))),
        ("opening", fixture("stream-open")["opening"].clone()),
    ] {
        let mut value_to_test = fixture("allocated");
        value_to_test[key] = value;
        rejects(&value_to_test);
    }
    for path in [
        "/owner_ref",
        "/source_refs/0",
        "/facts/0",
        "/facts/0/resource",
        "/facts/0/allocation",
        "/facts/0/audit_time",
        "/facts/0/coverage",
        "/facts/0/coverage/cpu",
    ] {
        let mut value = fixture("allocated");
        value.pointer_mut(path).unwrap()["unknown"] = json!(true);
        rejects(&value);
    }
    let mut value = fixture("allocated");
    value["facts"][0]["quantities"]["compute_uptime_ms"] = json!("60000");
    rejects(&value);
    value = fixture("allocated");
    value["owner_ref"]["kind"] = json!("unresolved-owner-kind");
    assert!(decode(&value).is_ok());
}

#[test]
fn required_nulls_cannot_be_omitted() {
    for (name, path, field) in [
        ("stream-open", "", "previous_event_digest"),
        ("measured", "/facts/0", "allocation"),
        ("allocated", "/facts/0/coverage/cpu", "missing_reason"),
    ] {
        let mut value = fixture(name);
        value
            .pointer_mut(path)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove(field);
        rejects(&value);
    }
}

#[test]
fn duplicate_keys_are_rejected_before_serde_or_canonicalization_can_erase_them() {
    let event = serde_json::to_string(&fixture("allocated")).unwrap();
    for raw in [
        event.replacen(
            "\"kind\":\"usage\"",
            "\"kind\":\"usage\",\"kind\":\"usage\"",
            1,
        ),
        event.replacen(
            "\"issuer\":\"nyxid-example\"",
            "\"issuer\":\"nyxid-example\",\"issuer\":\"other\"",
            1,
        ),
        event.replacen(
            "\"sandbox_cpu_millis_seconds\":\"120000\"",
            "\"sandbox_cpu_millis_seconds\":\"120000\",\"sandbox_cpu_millis_seconds\":\"120000\"",
            1,
        ),
        event.replacen(
            "\"sequence\":\"2\"",
            "\"sequence\":\"2\",\"sequen\\u0063e\":\"2\"",
            1,
        ),
    ] {
        assert!(serde_json::from_str::<Value>(&raw).is_ok());
        assert_eq!(
            decode_event(raw.as_bytes(), clock()).err(),
            Some("invalid_json")
        );
    }
}

#[test]
fn body_depth_trailing_bytes_and_json_number_spellings_are_bounded() {
    assert!(decode_event(&vec![b' '; 512 * 1024 + 1], clock()).is_err());
    assert!(
        decode_event(
            format!("{}0{}", "[".repeat(140), "]".repeat(140)).as_bytes(),
            clock()
        )
        .is_err()
    );
    let raw = serde_json::to_string(&fixture("allocated")).unwrap();
    assert!(decode_event(format!("{raw} {{}}").as_bytes(), clock()).is_err());
    for number in [
        "1.0",
        "1e0",
        "-1",
        "-0",
        "9007199254740993",
        "18446744073709551616",
    ] {
        let raw = raw.replacen(
            "\"schema_version\":1",
            &format!("\"schema_version\":{number}"),
            1,
        );
        assert!(decode_event(raw.as_bytes(), clock()).is_err());
    }
}

#[test]
fn decimal_bounds_and_exact_units_never_use_floating_point() {
    let event = decode(&fixture("measured")).unwrap();
    let Payload::Usage { facts } = event.payload() else {
        panic!("usage");
    };
    assert_eq!(
        facts[0].quantities[&Metric::MeasuredMemory].unwrap().get(),
        9_007_199_254_740_993
    );
    let mut value = fixture("measured");
    for malformed in [
        "", "00", "01", "+1", "-1", "1.0", "1e6", " 1", "1 ", "NaN", "９",
    ] {
        value["facts"][0]["quantities"]["sandbox_cpu_core_nanoseconds_v1"] = json!(malformed);
        rejects(&value);
    }
    value["facts"][0]["quantities"]["sandbox_cpu_core_nanoseconds_v1"] = json!("9".repeat(38));
    assert!(decode(&value).is_ok());
    value["facts"][0]["quantities"]["sandbox_cpu_core_nanoseconds_v1"] = json!("1".repeat(39));
    rejects(&value);
    for invalid in [json!(1), json!(1.5), json!(-1), json!(true)] {
        value["facts"][0]["quantities"]["sandbox_cpu_core_nanoseconds_v1"] = invalid;
        rejects(&value);
    }
    for count in ["0", "9223372036854775808", "01"] {
        value = fixture("allocated");
        value["sequence"] = json!(count);
        rejects(&value);
    }
    value = fixture("allocated");
    value["sequence"] = json!(i64::MAX.to_string());
    assert!(decode(&value).is_ok());
    assert_eq!(Metric::AllocatedCpu.units_per_hour(), 3_600_000);
    assert_eq!(Metric::AllocatedMemory.units_per_hour(), 3_686_400);
    assert_eq!(Metric::AllocatedDisk.units_per_hour(), 3_600);
    assert_eq!(Metric::MeasuredCpu.units_per_hour(), 3_600_000_000_000);
    assert_eq!(
        Metric::MeasuredMemory.units_per_hour(),
        3_865_470_566_400_000
    );
    assert_eq!(Metric::MeasuredDisk.units_per_hour(), 3_865_470_566_400_000);
}

#[test]
fn time_and_allocation_arithmetic_fail_before_pricing() {
    for (path, v) in [
        ("/facts/0/bucket_start_ms", json!("1795132800001")),
        ("/facts/0/start_ms", json!("1795132799999")),
        ("/facts/0/end_ms", json!("1795132860001")),
        ("/facts/0/end_ms", json!("1795132800000")),
        ("/facts/0/end_ms", json!("1795132859999")),
        ("/facts/0/audit_time/compute_uptime_ms", json!("60001")),
        ("/facts/0/allocation/cpu_millis", json!("9".repeat(38))),
        (
            "/facts/0/quantities/sandbox_cpu_millis_seconds",
            json!("120001"),
        ),
        ("/produced_at_ms", json!("9223372036854775808")),
        ("/observed_at_ms", json!("1795132799999")),
        ("/source_cutoff_ms", json!("1795132859999")),
        ("/produced_at_ms", json!((NOW + 1).to_string())),
    ] {
        let mut value = fixture("allocated");
        *value.pointer_mut(path).unwrap() = v;
        rejects(&value);
    }
    let value = fixture("allocated");
    let bytes = serde_json::to_vec(&value).unwrap();
    assert!(
        decode_event(
            &bytes,
            Clock {
                now_ms: 1_795_132_859_999,
                allowed_clock_skew_ms: 5000
            }
        )
        .is_err()
    );
    assert!(
        decode_event(
            &bytes,
            Clock {
                now_ms: u64::MAX,
                allowed_clock_skew_ms: 1
            }
        )
        .is_err()
    );
    assert!(
        decode_event(
            &bytes,
            Clock {
                now_ms: 1_795_132_860_000,
                allowed_clock_skew_ms: 1000
            }
        )
        .is_ok()
    );
}

#[test]
fn missing_partial_and_concurrent_binding_coverage_are_distinct() {
    assert!(decode(&fixture("measured-correction")).is_ok());
    for (path, v) in [
        ("/facts/0/coverage/cpu/covered_binding_ms", json!("120001")),
        ("/facts/0/coverage/memory/quality", json!("missing")),
        ("/facts/0/coverage/memory/quality", json!("partial")),
        ("/facts/0/coverage/memory/quality", json!("estimated")),
        (
            "/facts/0/quantities/sandbox_memory_working_set_byte_milliseconds_v1",
            Value::Null,
        ),
    ] {
        let mut value = fixture("measured");
        *value.pointer_mut(path).unwrap() = v;
        rejects(&value);
    }
    let mut value = fixture("measured-correction");
    value["correction"]["replacement"]["quantities"]["sandbox_memory_working_set_byte_milliseconds_v1"] =
        json!("0");
    rejects(&value);
    value = fixture("measured");
    value["facts"][0]["coverage"]["disk"]["expected_binding_ms"] = json!("0");
    value["facts"][0]["coverage"]["disk"]["covered_binding_ms"] = json!("0");
    assert!(decode(&value).is_ok());
    value["facts"][0]["quantities"]["sandbox_disk_used_byte_milliseconds_v1"] = json!("1");
    rejects(&value);
}

#[test]
fn terminal_compute_is_zero_while_home_capacity_remains() {
    let mut value = fixture("allocated");
    let fact = &mut value["facts"][0];
    fact["audit_time"]["compute_uptime_ms"] = json!("0");
    for dimension in ["cpu", "memory"] {
        fact["coverage"][dimension]["covered_binding_ms"] = json!("0");
        fact["coverage"][dimension]["expected_binding_ms"] = json!("0");
    }
    fact["quantities"]["sandbox_cpu_millis_seconds"] = json!("0");
    fact["quantities"]["sandbox_memory_mib_seconds"] = json!("0");
    assert!(decode(&value).is_ok());
    assert_eq!(
        value["facts"][0]["quantities"]["sandbox_disk_gib_seconds"],
        "600"
    );
}

#[test]
fn deterministic_array_order_duplicate_facts_and_binding_overlap() {
    let mut value = fixture("allocated");
    let second = value["source_refs"][0].clone();
    value["source_refs"].as_array_mut().unwrap().push(second);
    rejects(&value);
    value["source_refs"][1]["part"] = json!("1");
    assert!(decode(&value).is_ok());
    value["source_refs"].as_array_mut().unwrap().reverse();
    rejects(&value);
    value = fixture("allocated");
    let second = value["facts"][0].clone();
    value["facts"].as_array_mut().unwrap().push(second);
    rejects(&value);
    value["facts"][1]["fact_id"] = json!("00000000-0000-4000-8000-000000000065");
    rejects(&value);
    value["facts"][1]["resource"]["runtime_id"] = json!("runtime-other");
    rejects(&value);
    value["facts"][1]["resource"]["home_id"] = json!("home-other");
    assert!(decode(&value).is_ok());
    value["facts"].as_array_mut().unwrap().reverse();
    rejects(&value);
    value = fixture("allocated");
    value["facts"] = json!(vec![value["facts"][0].clone(); 65]);
    rejects(&value);
    value = fixture("allocated");
    value["source_refs"] = json!(vec![value["source_refs"][0].clone(); 65]);
    rejects(&value);
    value = fixture("allocated");
    value["facts"] = json!([]);
    assert!(decode(&value).is_ok());
}

#[test]
fn stream_control_baselines_and_uuid_versions_are_checked() {
    let mut value = fixture("stream-open");
    value["sequence"] = json!("2");
    rejects(&value);
    value = fixture("allocated");
    value["previous_event_digest"] = Value::Null;
    rejects(&value);
    value = fixture("stream-open");
    value["opening"]["capture_start_ms"] = json!("1795132801000");
    rejects(&value);
    value["opening"]["complete_lifecycle"] = json!(false);
    assert!(decode(&value).is_ok());
    value["opening"]["historical_missing_ranges"] =
        json!([{"start_ms":"1795132800000","end_ms":"1795132802000"}]);
    rejects(&value);
    for bad in [
        "bad",
        "00000000-0000-1000-8000-000000000001",
        "00000000-0000-4000-0000-000000000001",
    ] {
        value = fixture("allocated");
        value["stream_id"] = json!(bad);
        rejects(&value);
    }
    value = fixture("stream-seal");
    value["seal"]["reason"] = json!("compute_stopped");
    rejects(&value);
    value = fixture("stream-seal");
    value["seal"]["final_source"]["revision"] = json!("43");
    rejects(&value);
}

#[test]
fn exact_body_fact_and_source_bounds_are_accepted_before_overflow_is_rejected() {
    let mut value = fixture("allocated");
    let template = value["facts"][0].clone();
    value["facts"] = json!(
        (0..64)
            .map(|index| {
                let mut fact = template.clone();
                fact["fact_id"] = json!(format!("00000000-0000-4000-8000-{index:012x}"));
                fact["resource"]["runtime_id"] = json!(format!("runtime-{index}"));
                fact["resource"]["home_id"] = json!(format!("home-{index}"));
                fact
            })
            .collect::<Vec<_>>()
    );
    let source = value["source_refs"][0].clone();
    value["source_refs"] = json!(
        (0..64)
            .map(|index| {
                let mut source = source.clone();
                source["part"] = json!(index.to_string());
                source
            })
            .collect::<Vec<_>>()
    );
    let mut bytes = serde_json::to_vec(&value).unwrap();
    bytes.resize(512 * 1024, b' ');
    assert!(decode_event(&bytes, clock()).is_ok());
    bytes.push(b' ');
    assert_eq!(decode_event(&bytes, clock()).err(), Some("body_too_large"));
    value["source_refs"].as_array_mut().unwrap().swap(9, 10);
    rejects(&value);
}

#[test]
fn text_identity_and_digest_encodings_are_strict() {
    for value in [
        "".to_owned(),
        " a".to_owned(),
        "a ".to_owned(),
        "a\u{0080}b".to_owned(),
        "é".repeat(129),
    ] {
        let mut event = fixture("allocated");
        event["owner_ref"]["subject"] = json!(value);
        rejects(&event);
    }
    for value in ["A".repeat(64), "a".repeat(63), "g".repeat(64)] {
        let mut event = fixture("allocated");
        event["source_digest"] = json!(value);
        rejects(&event);
    }
    let mut event = fixture("allocated");
    event["owner_ref"]["subject"] = json!("é".repeat(128));
    assert!(decode(&event).is_ok());
}

#[test]
fn final_partial_minute_in_supported_timestamp_range_is_valid() {
    let mut value = fixture("measured");
    let end = i64::MAX as u64;
    let bucket = end / 60_000 * 60_000;
    for field in ["source_cutoff_ms", "observed_at_ms", "produced_at_ms"] {
        value[field] = json!(end.to_string());
    }
    let fact = &mut value["facts"][0];
    fact["bucket_start_ms"] = json!(bucket.to_string());
    fact["start_ms"] = json!(bucket.to_string());
    fact["end_ms"] = json!(end.to_string());
    fact["audit_time"]["compute_uptime_ms"] = json!((end - bucket).to_string());
    fact["audit_time"]["retained_home_ms"] = json!((end - bucket).to_string());
    assert!(
        decode_event(
            &serde_json::to_vec(&value).unwrap(),
            Clock {
                now_ms: end,
                allowed_clock_skew_ms: 0,
            }
        )
        .is_ok()
    );
}
