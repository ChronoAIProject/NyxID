use super::completeness_contract::{Coverage, Statement};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::PathBuf};

const MANIFEST_SHA256: &str = "d7bbfa8fda130be34acdc19c39ac0cba60a0a50a38a149bbf3e2c28c0248780d";

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cma_minute_completeness_v1")
}

#[test]
fn completeness_statement_manifest_and_contract() {
    let base = root();
    let manifest_bytes = fs::read(base.join("manifest.json")).unwrap();
    assert_eq!(sha(&manifest_bytes), MANIFEST_SHA256);
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let mut hashes = BTreeMap::new();
    for item in manifest["fixtures"].as_array().unwrap() {
        let filename = item["file"].as_str().unwrap();
        let bytes = fs::read(base.join(filename)).unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(serde_json_canonicalizer::to_vec(&value).unwrap(), bytes);
        assert_eq!(sha(&bytes), item["sha256"]);
        assert_eq!(
            Statement::from_value(value).is_ok(),
            item["valid"].as_bool().unwrap(),
            "{filename}"
        );
        hashes.insert(filename, sha(&bytes));
    }
    assert_eq!(
        sha(&serde_json_canonicalizer::to_vec(&hashes).unwrap()),
        manifest["bundle_sha256"]
    );
}

#[test]
fn observed_zero_and_missing_are_distinct_contributors() {
    let bytes = fs::read(root().join("measured-missing-zero.canonical.json")).unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    let statement = Statement::from_value(value).unwrap();
    assert_eq!(statement.facts.len(), 1);
    assert_eq!(statement.contributions.len(), 3);
    assert!(
        statement
            .contributions
            .iter()
            .any(|c| c.coverage == Coverage::Missing)
    );
    assert!(
        statement
            .contributions
            .iter()
            .any(|c| c.coverage == Coverage::ObservedZero)
    );
}
