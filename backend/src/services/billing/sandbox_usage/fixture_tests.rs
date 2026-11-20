use super::{
    contract::{decode_event, decode_receipt},
    contract_tests::{clock, decode, fixture},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, path::PathBuf};

// CMA copies this release bundle verbatim and pins this same manifest hash.
const MANIFEST_SHA256: &str = "2f189190dd24595d0b261a56030be38d2328df8c285d35fec08c3e3293948797";
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cma_usage_v1")
}
fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn manifest() -> Value {
    fixture("manifest")
}

#[test]
fn release_manifest_pins_every_schema_fixture_and_canonical_byte() {
    let bytes = fs::read(root().join("manifest.json")).unwrap();
    assert_eq!(sha(&bytes), MANIFEST_SHA256);
    let manifest = manifest();
    assert_eq!(manifest["contract"], "cma-usage-v1");
    assert_eq!(manifest["canonicalization"], "RFC8785");
    let files = manifest["files"].as_object().unwrap();
    let actual: BTreeSet<_> = fs::read_dir(root())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name != "manifest.json")
        .collect();
    assert_eq!(actual, files.keys().cloned().collect());
    for (name, hash) in files {
        assert_eq!(
            sha(&fs::read(root().join(name)).unwrap()),
            hash.as_str().unwrap(),
            "{name}"
        );
    }
    assert_eq!(
        sha(&serde_json_canonicalizer::to_vec(files).unwrap()),
        manifest["bundle_sha256"]
    );
}

#[test]
fn shared_schema_and_decoder_accept_all_golden_fixtures_with_exact_digests() {
    let schema = fixture("schema");
    let validator = jsonschema::validator_for(&schema).unwrap();
    for entry in manifest()["fixtures"].as_array().unwrap() {
        let bytes = fs::read(root().join(entry["file"].as_str().unwrap())).unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(validator.is_valid(&value), "{}", entry["name"]);
        let canonical = fs::read(root().join(entry["canonical_file"].as_str().unwrap())).unwrap();
        assert_eq!(serde_json_canonicalizer::to_vec(&value).unwrap(), canonical);
        assert_eq!(sha(&canonical), entry["canonical_sha256"]);
        if entry["kind"] == "event" {
            let event = decode_event(&bytes, clock()).unwrap();
            assert_eq!(event.canonical_bytes(), canonical);
            assert_eq!(event.digest().as_str(), entry["canonical_sha256"]);
        } else {
            let receipt = decode_receipt(&bytes, clock()).unwrap();
            assert!(receipt.matches(&decode(&fixture("allocated")).unwrap()));
            assert_eq!(
                serde_json_canonicalizer::to_vec(&receipt).unwrap(),
                canonical
            );
        }
    }
}

fn reverse_object_keys(value: &Value) -> String {
    match value {
        Value::Object(object) => format!(
            "{{ {} }}",
            object
                .iter()
                .rev()
                .map(|(key, value)| format!(
                    "{} : {}",
                    serde_json::to_string(key).unwrap(),
                    reverse_object_keys(value)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Array(values) => format!(
            "[ {} ]",
            values
                .iter()
                .map(reverse_object_keys)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => serde_json::to_string(other).unwrap(),
    }
}

#[test]
fn object_order_whitespace_and_equivalent_escapes_preserve_event_digest() {
    for name in [
        "stream-open",
        "allocated",
        "measured",
        "correction",
        "stream-seal",
    ] {
        let value = fixture(name);
        let before = decode(&value).unwrap();
        let reordered = reverse_object_keys(&value).replace("cma-example", "cma-\\u0065xample");
        let after = decode_event(reordered.as_bytes(), clock()).unwrap();
        assert_eq!(before.canonical_bytes(), after.canonical_bytes());
        assert_eq!(before.digest(), after.digest());
    }
    let mut value = fixture("allocated");
    let before = decode(&value).unwrap();
    value["previous_event_digest"] = json!("0".repeat(64));
    assert_ne!(before.digest(), decode(&value).unwrap().digest());
    value = fixture("allocated");
    value["event_id"] = json!("00000000-0000-4000-8000-000000000099");
    assert_ne!(before.digest(), decode(&value).unwrap().digest());
}

#[test]
fn rfc8785_utf16_order_and_string_escaping_match_independent_golden_bytes() {
    // RFC 8785 section 3.2.3: UTF-16 order puts the astral key before U+FB33.
    let input = json!({"\u{20ac}":"Euro", "\r":"CR", "\u{fb33}":"Hebrew", "1":"One",
        "\u{1f600}":"Emoji", "\u{0080}":"Control", "\u{00f6}":"Latin"});
    let expected = "{\"\\r\":\"CR\",\"1\":\"One\",\"\u{0080}\":\"Control\",\"ö\":\"Latin\",\"€\":\"Euro\",\"😀\":\"Emoji\",\"דּ\":\"Hebrew\"}";
    assert_eq!(
        serde_json_canonicalizer::to_vec(&input).unwrap(),
        expected.as_bytes()
    );
    let input = json!({"s":"\u{000f}\n\"\\/é"});
    assert_eq!(
        serde_json_canonicalizer::to_vec(&input).unwrap(),
        "{\"s\":\"\\u000f\\n\\\"\\\\/é\"}".as_bytes()
    );
    let raw = br#"{"x":1,"x":2}"#;
    assert!(serde_json::from_slice::<Value>(raw).is_ok());
    assert!(super::strict_json::parse(raw).is_err());
    assert!(super::strict_json::parse(br#"{"x":9007199254740992}"#).is_err());
    assert!(super::strict_json::parse(br#"{"x":"\ud800"}"#).is_err());
}

#[test]
fn schema_and_decoder_reject_unknown_shapes_versions_and_numeric_bounds() {
    let schema = fixture("schema");
    let validator = jsonschema::validator_for(&schema).unwrap();
    for (path, replacement) in [
        ("/schema_version", json!(2)),
        ("/kind", json!("batch")),
        ("/algorithm_version", json!("latest")),
        ("/metric_definition_version", json!("measured-v2")),
        ("/sequence", json!("9223372036854775808")),
        ("/sequence", json!("0")),
        ("/source_refs/0/part", json!("9223372036854775808")),
        ("/facts/0/allocation", json!({})),
        (
            "/facts/0/quantities/sandbox_cpu_core_nanoseconds_v1",
            json!("01"),
        ),
        (
            "/facts/0/quantities/sandbox_cpu_core_nanoseconds_v1",
            json!("9".repeat(39)),
        ),
        (
            "/facts/0/quantities/sandbox_cpu_core_nanoseconds_v1",
            json!(123),
        ),
        ("/source_cutoff_ms", json!("-1")),
        ("/event_id", json!("not-a-uuid")),
    ] {
        let mut value = fixture("measured");
        *value.pointer_mut(path).unwrap() = replacement;
        assert!(!validator.is_valid(&value), "{path}");
        assert!(decode(&value).is_err(), "{path}");
    }
}

#[test]
fn measured_residual_or_missing_evidence_cannot_overlap_at_zero_logical_uptime() {
    for name in ["measured", "measured-correction"] {
        let mut value = fixture("measured");
        let mut fact = if name == "measured" {
            value["facts"][0].clone()
        } else {
            fixture(name)["correction"]["replacement"].clone()
        };
        fact["evidence_revision"] = json!("1");
        fact["audit_time"]["compute_uptime_ms"] = json!("0");
        fact["audit_time"]["retained_home_ms"] = json!("0");
        value["facts"] = json!([fact.clone()]);
        assert!(decode(&value).is_ok());
        fact["fact_id"] = json!("00000000-0000-4000-8000-000000000065");
        value["facts"].as_array_mut().unwrap().push(fact);
        assert!(decode(&value).is_err());
        value["facts"][1]["resource"]["runtime_id"] = json!("other-runtime");
        assert!(decode(&value).is_err());
        value["facts"][1]["resource"]["home_id"] = json!("other-home");
        assert!(decode(&value).is_ok());
    }
}
