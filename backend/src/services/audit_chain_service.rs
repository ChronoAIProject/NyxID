use std::collections::BTreeMap;

use chrono::{DateTime, TimeZone, Utc};
use futures::TryStreamExt;
use hmac::{Hmac, Mac};
use mongodb::bson::doc;
use rand::Rng;
use serde::Serialize;
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::errors::{AppError, AppResult};
use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};

type HmacSha256 = Hmac<Sha256>;

pub const GENESIS_PREV_HASH: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";
const RECORD_SEPARATOR: u8 = 0x1e;
const MAX_APPEND_ATTEMPTS: usize = 8;
pub const DEFAULT_VERIFY_LIMIT: i64 = 10_000;
pub const MAX_VERIFY_LIMIT: i64 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditChainBreakKind {
    Gap,
    LinkMismatch,
    HashMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditChainBreak {
    pub break_seq: i64,
    pub break_kind: AuditChainBreakKind,
    pub expected: String,
    pub actual: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditChainStatus {
    Ok,
    Broken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditChainVerifyReport {
    pub status: AuditChainStatus,
    pub checked_count: u64,
    pub pre_chain_count: u64,
    pub head_seq: Option<i64>,
    pub head_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub break_info: Option<AuditChainBreak>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_from_seq: Option<i64>,
}

#[derive(Debug, Clone)]
struct ChainTail {
    seq: i64,
    entry_hash: String,
}

pub fn compute_entry_hash(entry: &AuditLog, key: &[u8]) -> AppResult<String> {
    let canonical = canonical_audit_bytes(entry)?;
    Ok(hmac_sha256_hex(key, &canonical))
}

pub fn prepare_chained_entry(
    entry: &mut AuditLog,
    seq: i64,
    prev_hash: String,
    key: &[u8],
) -> AppResult<()> {
    entry.created_at = truncate_to_bson_millis(entry.created_at);
    entry.seq = Some(seq);
    entry.prev_hash = Some(prev_hash);
    entry.entry_hash = None;
    let entry_hash = compute_entry_hash(entry, key)?;
    entry.entry_hash = Some(entry_hash);
    Ok(())
}

pub async fn append_chained_entry(
    db: &mongodb::Database,
    mut entry: AuditLog,
    key: &[u8],
) -> Result<AuditLog, mongodb::error::Error> {
    let collection = db.collection::<AuditLog>(AUDIT_LOG);
    let mut last_error = None;

    for attempt in 1..=MAX_APPEND_ATTEMPTS {
        let tail = read_tail(db).await?;
        let (seq, prev_hash) = match tail {
            Some(tail) => (tail.seq + 1, tail.entry_hash),
            None => (1, GENESIS_PREV_HASH.to_string()),
        };

        if let Err(error) = prepare_chained_entry(&mut entry, seq, prev_hash, key) {
            return Err(mongodb::error::Error::custom(error.to_string()));
        }

        match collection.insert_one(&entry).await {
            Ok(_) => return Ok(entry),
            Err(error) if is_duplicate_key_error(&error) && attempt < MAX_APPEND_ATTEMPTS => {
                last_error = Some(error);
                sleep_before_retry(attempt).await;
            }
            Err(error) => return Err(error),
        }
    }

    Err(last_error
        .unwrap_or_else(|| mongodb::error::Error::custom("audit chain append exhausted retries")))
}

pub async fn verify_chain(
    db: &mongodb::Database,
    key: &[u8],
    from_seq: Option<i64>,
    to_seq: Option<i64>,
    limit: Option<i64>,
) -> AppResult<AuditChainVerifyReport> {
    let from_seq = from_seq.unwrap_or(1);
    if from_seq < 1 {
        return Err(AppError::ValidationError(
            "from_seq must be greater than or equal to 1".to_string(),
        ));
    }

    if let Some(to_seq) = to_seq
        && to_seq < from_seq
    {
        return Err(AppError::ValidationError(
            "to_seq must be greater than or equal to from_seq".to_string(),
        ));
    }

    let limit = limit.unwrap_or(DEFAULT_VERIFY_LIMIT);
    if !(1..=MAX_VERIFY_LIMIT).contains(&limit) {
        return Err(AppError::ValidationError(format!(
            "limit must be between 1 and {MAX_VERIFY_LIMIT}"
        )));
    }

    let collection = db.collection::<AuditLog>(AUDIT_LOG);
    let pre_chain_count = collection
        .count_documents(doc! { "seq": { "$exists": false } })
        .await?;
    let tail = read_tail(db).await?;
    let head_seq = tail.as_ref().map(|tail| tail.seq);
    let head_hash = tail.map(|tail| tail.entry_hash);

    let mut filter = doc! { "seq": { "$gte": from_seq } };
    if let Some(to_seq) = to_seq {
        filter.insert("seq", doc! { "$gte": from_seq, "$lte": to_seq });
    }

    let entries: Vec<AuditLog> = collection
        .find(filter)
        .sort(doc! { "seq": 1 })
        .limit(limit)
        .await?
        .try_collect()
        .await?;

    let mut checked_count = 0_u64;
    let mut expected_seq = from_seq;
    let mut expected_prev_hash = if from_seq == 1 {
        GENESIS_PREV_HASH.to_string()
    } else {
        match load_entry_by_seq(db, from_seq - 1).await? {
            Some(previous) => previous.entry_hash.unwrap_or_default(),
            None => {
                return Ok(broken_report(
                    checked_count,
                    pre_chain_count,
                    head_seq,
                    head_hash,
                    AuditChainBreak {
                        break_seq: from_seq,
                        break_kind: AuditChainBreakKind::Gap,
                        expected: (from_seq - 1).to_string(),
                        actual: "missing".to_string(),
                    },
                ));
            }
        }
    };

    for entry in &entries {
        let actual_seq = entry.seq.unwrap_or_default();
        if actual_seq != expected_seq {
            return Ok(broken_report(
                checked_count,
                pre_chain_count,
                head_seq,
                head_hash,
                AuditChainBreak {
                    break_seq: expected_seq,
                    break_kind: AuditChainBreakKind::Gap,
                    expected: expected_seq.to_string(),
                    actual: actual_seq.to_string(),
                },
            ));
        }

        let actual_prev_hash = entry.prev_hash.clone().unwrap_or_default();
        if actual_prev_hash != expected_prev_hash {
            return Ok(broken_report(
                checked_count,
                pre_chain_count,
                head_seq,
                head_hash,
                AuditChainBreak {
                    break_seq: expected_seq,
                    break_kind: AuditChainBreakKind::LinkMismatch,
                    expected: expected_prev_hash,
                    actual: actual_prev_hash,
                },
            ));
        }

        let expected_entry_hash = compute_entry_hash(entry, key)?;
        let actual_entry_hash = entry.entry_hash.clone().unwrap_or_default();
        if actual_entry_hash != expected_entry_hash {
            return Ok(broken_report(
                checked_count,
                pre_chain_count,
                head_seq,
                head_hash,
                AuditChainBreak {
                    break_seq: expected_seq,
                    break_kind: AuditChainBreakKind::HashMismatch,
                    expected: expected_entry_hash,
                    actual: actual_entry_hash,
                },
            ));
        }

        checked_count += 1;
        expected_seq += 1;
        expected_prev_hash = actual_entry_hash;
    }

    let next_from_seq = if entries.len() as i64 == limit
        && to_seq.is_none_or(|to_seq| expected_seq <= to_seq)
        && head_seq.is_some_and(|head_seq| expected_seq <= head_seq)
    {
        Some(expected_seq)
    } else {
        None
    };

    Ok(AuditChainVerifyReport {
        status: AuditChainStatus::Ok,
        checked_count,
        pre_chain_count,
        head_seq,
        head_hash,
        break_info: None,
        next_from_seq,
    })
}

async fn read_tail(db: &mongodb::Database) -> Result<Option<ChainTail>, mongodb::error::Error> {
    let tail = db
        .collection::<AuditLog>(AUDIT_LOG)
        .find(doc! { "seq": { "$exists": true } })
        .sort(doc! { "seq": -1 })
        .limit(1)
        .await?
        .try_next()
        .await?;

    Ok(tail.and_then(|entry| {
        Some(ChainTail {
            seq: entry.seq?,
            entry_hash: entry.entry_hash?,
        })
    }))
}

async fn load_entry_by_seq(
    db: &mongodb::Database,
    seq: i64,
) -> Result<Option<AuditLog>, mongodb::error::Error> {
    db.collection::<AuditLog>(AUDIT_LOG)
        .find_one(doc! { "seq": seq })
        .await
}

fn broken_report(
    checked_count: u64,
    pre_chain_count: u64,
    head_seq: Option<i64>,
    head_hash: Option<String>,
    break_info: AuditChainBreak,
) -> AuditChainVerifyReport {
    AuditChainVerifyReport {
        status: AuditChainStatus::Broken,
        checked_count,
        pre_chain_count,
        head_seq,
        head_hash,
        break_info: Some(break_info),
        next_from_seq: None,
    }
}

fn canonical_audit_bytes(entry: &AuditLog) -> AppResult<Vec<u8>> {
    let seq = entry
        .seq
        .ok_or_else(|| AppError::Internal("audit chain entry missing seq".to_string()))?;
    let prev_hash = entry
        .prev_hash
        .as_deref()
        .ok_or_else(|| AppError::Internal("audit chain entry missing prev_hash".to_string()))?;
    let event_data = entry
        .event_data
        .as_ref()
        .map(canonical_json_string)
        .transpose()?
        .unwrap_or_default();
    let created_at_millis = entry.created_at.timestamp_millis();

    let fields = [
        seq.to_string(),
        prev_hash.to_string(),
        entry.id.clone(),
        entry.user_id.clone().unwrap_or_default(),
        entry.event_type.clone(),
        event_data,
        entry.ip_address.clone().unwrap_or_default(),
        entry.user_agent.clone().unwrap_or_default(),
        entry.api_key_id.clone().unwrap_or_default(),
        entry.api_key_name.clone().unwrap_or_default(),
        created_at_millis.to_string(),
    ];

    Ok(join_record_fields(&fields))
}

fn join_record_fields(fields: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            out.push(RECORD_SEPARATOR);
        }
        out.extend_from_slice(field.as_bytes());
    }
    out
}

fn canonical_json_string(value: &serde_json::Value) -> AppResult<String> {
    serde_json::to_string(&canonical_json_value(value))
        .map_err(|error| AppError::Internal(format!("failed to canonicalize audit JSON: {error}")))
}

fn canonical_json_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(canonical_json_value).collect())
        }
        serde_json::Value::Object(map) => {
            let sorted: BTreeMap<String, serde_json::Value> = map
                .iter()
                .map(|(key, value)| (key.clone(), canonical_json_value(value)))
                .collect();
            serde_json::Value::Object(sorted.into_iter().collect())
        }
        _ => value.clone(),
    }
}

fn hmac_sha256_hex(key: &[u8], input: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC-SHA256 accepts any key length");
    mac.update(input);
    hex::encode(mac.finalize().into_bytes())
}

fn truncate_to_bson_millis(ts: DateTime<Utc>) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ts.timestamp_millis())
        .single()
        .expect("timestamp_millis from DateTime<Utc> is valid")
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    matches!(
        error.kind.as_ref(),
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write_error))
            if write_error.code == 11000
    )
}

async fn sleep_before_retry(attempt: usize) {
    let base_ms = 5_u64.saturating_mul(1_u64 << attempt.min(5));
    let jitter_ms = rand::thread_rng().gen_range(0..=10);
    tokio::time::sleep(std::time::Duration::from_millis(base_ms + jitter_ms)).await;
}

pub fn derive_audit_chain_hmac_key(
    env_override: Option<&str>,
    encryption_key: Option<&str>,
    jwt_private_key_pem: Option<&[u8]>,
) -> Zeroizing<[u8; 32]> {
    if let Some(raw) = env_override {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            match hex::decode(trimmed) {
                Ok(bytes) if bytes.len() == 32 => {
                    let mut out = [0u8; 32];
                    out.copy_from_slice(&bytes);
                    tracing::info!("audit-chain HMAC key loaded from AUDIT_CHAIN_HMAC_KEY");
                    return Zeroizing::new(out);
                }
                _ => {
                    panic!("AUDIT_CHAIN_HMAC_KEY must be 64 hex characters (32 bytes)");
                }
            }
        }
    }

    let jwt_private_key_pem = jwt_private_key_pem.unwrap_or_default();
    if let Some(master) = decode_hmac_seed(encryption_key) {
        tracing::info!("audit-chain HMAC key derived from ENCRYPTION_KEY");
        return crate::crypto::hmac_keys::derive_hmac_key(
            "audit-chain",
            Some(&master),
            jwt_private_key_pem,
        );
    }

    if !jwt_private_key_pem.is_empty() {
        tracing::info!("audit-chain HMAC key derived from JWT private key");
        return crate::crypto::hmac_keys::derive_hmac_key("audit-chain", None, jwt_private_key_pem);
    }

    panic!(
        "audit-chain HMAC key has no source: AUDIT_CHAIN_HMAC_KEY, ENCRYPTION_KEY, \
         and the JWT private key are all missing."
    );
}

fn decode_hmac_seed(encryption_key: Option<&str>) -> Option<Zeroizing<Vec<u8>>> {
    let hex_key = encryption_key?;
    let Ok(master) = hex::decode(hex_key.trim()) else {
        return None;
    };
    if master.len() == 32 {
        Some(Zeroizing::new(master))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use mongodb::bson::{Document, doc};

    fn test_entry(event_data: Option<serde_json::Value>) -> AuditLog {
        AuditLog {
            id: "entry-1".to_string(),
            user_id: Some("user-1".to_string()),
            event_type: "login".to_string(),
            event_data,
            ip_address: Some("127.0.0.1".to_string()),
            user_agent: Some("agent".to_string()),
            api_key_id: None,
            api_key_name: None,
            seq: Some(1),
            prev_hash: Some(GENESIS_PREV_HASH.to_string()),
            entry_hash: None,
            created_at: Utc.timestamp_millis_opt(1_700_000_000_123).unwrap(),
        }
    }

    #[test]
    fn canonical_json_sorts_nested_object_keys() {
        let a = serde_json::json!({
            "b": 2,
            "a": {
                "z": true,
                "m": [ { "y": 1, "x": 2 } ],
            },
        });
        let b = serde_json::json!({
            "a": {
                "m": [ { "x": 2, "y": 1 } ],
                "z": true,
            },
            "b": 2,
        });

        assert_eq!(
            canonical_json_string(&a).expect("canonical a"),
            canonical_json_string(&b).expect("canonical b")
        );
        assert_eq!(
            canonical_json_string(&a).expect("canonical"),
            r#"{"a":{"m":[{"x":2,"y":1}],"z":true},"b":2}"#
        );
    }

    #[test]
    fn canonical_audit_bytes_include_empty_optional_fields_and_millis() {
        let entry = AuditLog {
            user_id: None,
            event_data: None,
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
            ..test_entry(None)
        };

        let bytes = canonical_audit_bytes(&entry).expect("canonical bytes");
        let fields: Vec<&[u8]> = bytes.split(|byte| *byte == RECORD_SEPARATOR).collect();
        assert_eq!(fields.len(), 11);
        assert_eq!(fields[0], b"1");
        assert_eq!(fields[2], b"entry-1");
        assert_eq!(fields[3], b"");
        assert_eq!(fields[5], b"");
        assert_eq!(fields[10], b"1700000000123");
    }

    #[test]
    fn entry_hash_is_stable_for_key_sorted_json() {
        let key = [0x42_u8; 32];
        let mut a = test_entry(Some(serde_json::json!({ "z": 1, "a": 2 })));
        let mut b = test_entry(Some(serde_json::json!({ "a": 2, "z": 1 })));

        prepare_chained_entry(&mut a, 1, GENESIS_PREV_HASH.to_string(), &key).expect("prepare a");
        prepare_chained_entry(&mut b, 1, GENESIS_PREV_HASH.to_string(), &key).expect("prepare b");

        assert_eq!(a.entry_hash, b.entry_hash);
        assert_eq!(a.entry_hash.as_deref().expect("hash").len(), 64);
    }

    #[test]
    fn entry_hash_changes_when_event_content_changes() {
        let key = [0x42_u8; 32];
        let mut a = test_entry(Some(serde_json::json!({ "a": 2 })));
        let mut b = test_entry(Some(serde_json::json!({ "a": 3 })));

        prepare_chained_entry(&mut a, 1, GENESIS_PREV_HASH.to_string(), &key).expect("prepare a");
        prepare_chained_entry(&mut b, 1, GENESIS_PREV_HASH.to_string(), &key).expect("prepare b");

        assert_ne!(a.entry_hash, b.entry_hash);
    }

    #[test]
    fn audit_chain_key_prefers_env_override() {
        let override_key = hex::encode([0xAA_u8; 32]);
        let enc = hex::encode([0x55_u8; 32]);
        let jwt = vec![0x33_u8; 512];

        let key = derive_audit_chain_hmac_key(Some(&override_key), Some(&enc), Some(&jwt));
        assert_eq!(key.as_slice(), &[0xAA_u8; 32]);
    }

    #[test]
    fn audit_chain_key_is_domain_separated_from_cli_pairing() {
        let enc = hex::encode([0x55_u8; 32]);
        let jwt = vec![0x33_u8; 512];

        let audit = derive_audit_chain_hmac_key(None, Some(&enc), Some(&jwt));
        let cli = crate::crypto::hmac_keys::derive_hmac_key(
            "cli-pairing",
            Some(&hex::decode(enc).expect("hex")),
            &jwt,
        );
        assert_ne!(audit.as_slice(), cli.as_slice());
    }

    #[tokio::test]
    async fn append_chained_entry_writes_contiguous_chain() {
        let Some(db) = crate::test_utils::connect_test_database("audit_chain_append").await else {
            return;
        };
        crate::db::ensure_indexes(&db).await.expect("indexes");

        let key = [0x77_u8; 32];
        for i in 0..3 {
            let entry = AuditLog {
                id: format!("entry-{i}"),
                user_id: Some("user".to_string()),
                event_type: "test".to_string(),
                event_data: Some(serde_json::json!({ "i": i })),
                ip_address: None,
                user_agent: None,
                api_key_id: None,
                api_key_name: None,
                seq: None,
                prev_hash: None,
                entry_hash: None,
                created_at: Utc::now(),
            };
            append_chained_entry(&db, entry, &key)
                .await
                .expect("append");
        }

        let report = verify_chain(&db, &key, None, None, None)
            .await
            .expect("verify");
        assert_eq!(report.status, AuditChainStatus::Ok);
        assert_eq!(report.checked_count, 3);
        assert_eq!(report.head_seq, Some(3));
    }

    #[tokio::test]
    async fn verify_chain_detects_field_tamper() {
        let Some(db) = crate::test_utils::connect_test_database("audit_chain_tamper").await else {
            return;
        };
        crate::db::ensure_indexes(&db).await.expect("indexes");

        let key = [0x77_u8; 32];
        let entry = AuditLog {
            id: "entry-1".to_string(),
            user_id: Some("user".to_string()),
            event_type: "test".to_string(),
            event_data: Some(serde_json::json!({ "ok": true })),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
            seq: None,
            prev_hash: None,
            entry_hash: None,
            created_at: Utc::now(),
        };
        append_chained_entry(&db, entry, &key)
            .await
            .expect("append");

        db.collection::<Document>(AUDIT_LOG)
            .update_one(
                doc! { "seq": 1 },
                doc! { "$set": { "event_data": { "ok": false } } },
            )
            .await
            .expect("tamper");

        let report = verify_chain(&db, &key, None, None, None)
            .await
            .expect("verify");
        assert_eq!(report.status, AuditChainStatus::Broken);
        let break_info = report.break_info.expect("break");
        assert_eq!(break_info.break_seq, 1);
        assert_eq!(break_info.break_kind, AuditChainBreakKind::HashMismatch);
    }

    #[tokio::test]
    async fn verify_chain_detects_mid_chain_deletion_gap() {
        let Some(db) = crate::test_utils::connect_test_database("audit_chain_gap").await else {
            return;
        };
        crate::db::ensure_indexes(&db).await.expect("indexes");

        let key = [0x77_u8; 32];
        for i in 0..3 {
            let entry = AuditLog {
                id: format!("entry-{i}"),
                user_id: None,
                event_type: "test".to_string(),
                event_data: Some(serde_json::json!({ "i": i })),
                ip_address: None,
                user_agent: None,
                api_key_id: None,
                api_key_name: None,
                seq: None,
                prev_hash: None,
                entry_hash: None,
                created_at: Utc::now(),
            };
            append_chained_entry(&db, entry, &key)
                .await
                .expect("append");
        }

        db.collection::<Document>(AUDIT_LOG)
            .delete_one(doc! { "seq": 2 })
            .await
            .expect("delete");

        let report = verify_chain(&db, &key, None, None, None)
            .await
            .expect("verify");
        assert_eq!(report.status, AuditChainStatus::Broken);
        let break_info = report.break_info.expect("break");
        assert_eq!(break_info.break_seq, 2);
        assert_eq!(break_info.break_kind, AuditChainBreakKind::Gap);
    }

    #[tokio::test]
    async fn verify_chain_counts_pre_chain_rows() {
        let Some(db) = crate::test_utils::connect_test_database("audit_chain_legacy").await else {
            return;
        };
        crate::db::ensure_indexes(&db).await.expect("indexes");

        db.collection::<AuditLog>(AUDIT_LOG)
            .insert_one(AuditLog {
                id: "legacy".to_string(),
                user_id: None,
                event_type: "legacy".to_string(),
                event_data: None,
                ip_address: None,
                user_agent: None,
                api_key_id: None,
                api_key_name: None,
                seq: None,
                prev_hash: None,
                entry_hash: None,
                created_at: Utc::now(),
            })
            .await
            .expect("legacy insert");

        let key = [0x77_u8; 32];
        let report = verify_chain(&db, &key, None, None, None)
            .await
            .expect("verify");
        assert_eq!(report.status, AuditChainStatus::Ok);
        assert_eq!(report.checked_count, 0);
        assert_eq!(report.pre_chain_count, 1);
    }
}
