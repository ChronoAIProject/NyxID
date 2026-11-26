//! Shared fixture compiled into both server binaries. Only test hashes, poll
//! handles and raw BSON rows from the task database are exchanged between processes.
use super::grant_tests::{KEY, fixture, input, selection};
use super::*;
use crate::crypto::token::hash_token;
use crate::models::api_key_credential;
use crate::services::session_authority::DerivedMfaAuthority;
use bson::Document;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Request {
    code: String,
    grant: bool,
    hashes: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Exchange {
    requests: Vec<Request>,
}

async fn issue(state: &crate::AppState, actor: &str, keys: &EncryptionKeys) -> Exchange {
    let db = &state.db;
    let mut requests = Vec::new();
    for (v2, grant) in [(false, false), (true, false), (true, true)] {
        let pending = if v2 {
            initiate_v2(db, KEY, InitiateInput::default(), true).await
        } else {
            initiate(db, KEY, InitiateInput::default()).await
        }
        .unwrap();
        let decision = input(actor, &pending.user_code);
        let authority = DerivedMfaAuthority::new(false, false);
        let hashes = if grant {
            approve_with_agent_key(db, keys, KEY, decision, selection(), None, authority)
                .await
                .unwrap();
            let row = collection_for_protocol(db, true)
                .find_one(doc! {
                    "device_code_hmac": hmac_hex(KEY, pending.device_code.as_bytes())
                })
                .await
                .unwrap()
                .unwrap();
            let child = db
                .collection::<Document>(api_key_credential::COLLECTION_NAME)
                .find_one(doc! {"_id": row.agent_key_grant.unwrap().credential_id})
                .await
                .unwrap()
                .unwrap();
            vec![child.get_str("secret_hash").unwrap().to_owned()]
        } else {
            approve(
                db,
                &state.config,
                &state.jwt_keys,
                keys,
                KEY,
                decision,
                authority,
            )
            .await
            .unwrap();
            let PollClaim::Account(account) =
                poll_internal(db, KEY, &pending.device_code, Some(keys), false)
                    .await
                    .unwrap()
            else {
                panic!("expected session")
            };
            vec![
                hash_token(&account.access_token),
                hash_token(&account.refresh_token),
            ]
        };
        requests.push(Request {
            code: pending.device_code,
            grant,
            hashes,
        });
    }
    Exchange { requests }
}

#[derive(Serialize, Deserialize)]
struct Snapshot {
    exchange: Exchange,
    collections: std::collections::BTreeMap<String, Vec<Document>>,
}

async fn save(db: &Database, exchange: Exchange, file: &std::path::Path) {
    use crate::models::{api_key, refresh_token, session, user};
    use futures::TryStreamExt;
    let mut collections = std::collections::BTreeMap::new();
    for name in [
        AUTH_DEVICE_CODES,
        V2_COLLECTION_NAME,
        user::COLLECTION_NAME,
        session::COLLECTION_NAME,
        refresh_token::COLLECTION_NAME,
        api_key::COLLECTION_NAME,
        api_key_credential::COLLECTION_NAME,
    ] {
        let rows = db
            .collection::<Document>(name)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        collections.insert(name.to_owned(), rows);
    }
    std::fs::write(
        file,
        bson::to_vec(&Snapshot {
            exchange,
            collections,
        })
        .unwrap(),
    )
    .unwrap();
}

async fn restore(db: &Database, file: &std::path::Path) -> Exchange {
    let snapshot: Snapshot = bson::from_slice(&std::fs::read(file).unwrap()).unwrap();
    for (name, rows) in snapshot.collections {
        if !rows.is_empty() {
            db.collection::<Document>(&name)
                .insert_many(rows)
                .await
                .unwrap();
        }
    }
    snapshot.exchange
}

async fn read(db: &Database, keys: &EncryptionKeys, exchange: &Exchange, reject: bool) {
    for request in &exchange.requests {
        let hashes = if request.grant {
            poll_agent_key(db, keys, KEY, &request.code)
                .await
                .map(|d| vec![hash_token(&d.credential)])
        } else {
            poll_and_prepare(db, keys, KEY, &request.code)
                .await
                .map(|claim| {
                    let PollClaim::Account(account) = claim else {
                        panic!("expected account")
                    };
                    vec![
                        hash_token(&account.access_token),
                        hash_token(&account.refresh_token),
                    ]
                })
        };
        let row = collection_for_device_code(db, &request.code)
            .find_one(doc! {
                "device_code_hmac": hmac_hex(KEY, request.code.as_bytes())
            })
            .await
            .unwrap()
            .unwrap();
        if reject {
            assert!(hashes.is_err());
            assert_eq!(row.status, AuthDeviceCodeStatus::Approved);
        } else {
            assert_eq!(hashes.unwrap(), request.hashes);
            assert!(matches!(
                poll_and_prepare(db, keys, KEY, &request.code)
                    .await
                    .unwrap(),
                PollClaim::AlreadyDelivered
            ));
            assert!(
                row.delivery_access_token_encrypted.is_none()
                    && row.delivery_refresh_token_encrypted.is_none()
            );
            assert!(
                row.agent_key_grant
                    .and_then(|g| g.delivery_credential_encrypted)
                    .is_none()
            );
        }
    }
}

#[tokio::test]
async fn mixed_device_session_and_agent_delivery() {
    let step = std::env::var("NYXID_RECORD_FIXTURE_STEP").unwrap_or_else(|_| "roundtrip".into());
    let file = std::env::var_os("NYXID_RECORD_FIXTURE_FILE");
    let (state, actor) = fixture("mixed_device_delivery").await;
    let mut config = state.config.clone();
    config.contextual_record_writes =
        std::env::var("CONTEXTUAL_RECORD_WRITES").as_deref() == Ok("true");
    config.allow_legacy_record_reads =
        std::env::var("ALLOW_LEGACY_RECORD_READS").as_deref() != Ok("false");
    let keys = EncryptionKeys::from_config(&config);
    match step.as_str() {
        "roundtrip" | "write" => {
            let exchange = issue(&state, &actor, &keys).await;
            if let Some(file) = file {
                save(&state.db, exchange, file.as_ref()).await;
            } else {
                read(&state.db, &keys, &exchange, false).await;
            }
        }
        "read" | "reject" => {
            let exchange = restore(&state.db, file.unwrap().as_ref()).await;
            read(&state.db, &keys, &exchange, step == "reject").await;
        }
        _ => panic!("unknown fixture operation"),
    }
}

// Candidate storage tests
#[tokio::test]
async fn migrate_retained_device_deliveries() {
    let (state, actor) = fixture("device_retained").await;
    let mut legacy = crate::test_utils::test_encryption_keys();
    legacy.set_contextual_record_writes(false);
    let file = std::env::var_os("NYXID_RECORD_FIXTURE_FILE");
    let exchange: Exchange = match file.as_ref() {
        Some(path) => restore(&state.db, path.as_ref()).await,
        None => issue(&state, &actor, &legacy).await,
    };
    let db = state.db.clone();
    let keys = &state.encryption_keys;
    for request in &exchange.requests {
        let collection = collection_for_device_code(&db, &request.code);
        let name = collection.name();
        let docs = db.collection::<Document>(name);
        let filter = doc! {"device_code_hmac": hmac_hex(KEY, request.code.as_bytes())};
        let before = docs.find_one(filter.clone()).await.unwrap().unwrap();
        assert!(!before.contains_key("delivery_context_version"));
        assert!(record::verify(keys, name, &before).await.is_err());
        assert!(record::migrate(&db, keys, name, &before).await.unwrap());
        assert!(!record::migrate(&db, keys, name, &before).await.unwrap());
        let sealed = docs.find_one(filter.clone()).await.unwrap().unwrap();
        record::verify(keys, name, &sealed).await.unwrap();
        assert_eq!(sealed.get("expires_at"), before.get("expires_at"));
        assert!(!record::migrate(&db, keys, name, &sealed).await.unwrap());
        let other = if name == AUTH_DEVICE_CODES {
            V2_COLLECTION_NAME
        } else {
            AUTH_DEVICE_CODES
        };
        assert!(record::verify(keys, other, &sealed).await.is_err());
        for key in ["_id", "approved_user_id"] {
            let mut swapped = sealed.clone();
            swapped.insert(key, "different");
            assert!(record::verify(keys, name, &swapped).await.is_err());
        }
        let mut conflicting = sealed.clone();
        conflicting.insert("approved_session_id", "other-session");
        docs.replace_one(filter.clone(), &conflicting)
            .await
            .unwrap();
        if request.grant {
            assert!(poll_agent_key(&db, keys, KEY, &request.code).await.is_err());
        } else {
            assert!(
                poll_and_prepare(&db, keys, KEY, &request.code)
                    .await
                    .is_err()
            );
        }
        assert_eq!(
            docs.find_one(filter.clone())
                .await
                .unwrap()
                .unwrap()
                .get_str("status"),
            Ok("approved")
        );
        docs.replace_one(filter, &sealed).await.unwrap();
        if request.grant {
            for key in ["api_key_id", "credential_id"] {
                let mut swapped = sealed.clone();
                swapped
                    .get_document_mut("agent_key_grant")
                    .unwrap()
                    .insert(key, "different");
                assert!(record::verify(keys, name, &swapped).await.is_err());
            }
        } else {
            let mut swapped = sealed.clone();
            swapped.insert(record::ACCESS, sealed.get(record::REFRESH).unwrap().clone());
            assert!(record::verify(keys, name, &swapped).await.is_err());
            let mut missing = sealed.clone();
            missing.remove(record::REFRESH);
            assert!(record::verify(keys, name, &missing).await.is_err());
            let mut wrong = sealed.clone();
            wrong.insert("approved_session_id", "different");
            assert!(record::verify(keys, name, &wrong).await.is_err());
        }
    }
    if let Some(path) = file {
        save(&db, exchange, path.as_ref()).await;
    } else {
        read(&db, keys, &exchange, false).await;
    }
}

#[tokio::test]
async fn migration_preserves_winning_claim_expiry_and_revocation() {
    let (state, actor) = fixture("device_storage_race").await;
    let db = &state.db;
    let mut legacy = crate::test_utils::test_encryption_keys();
    legacy.set_contextual_record_writes(false);
    let keys = &state.encryption_keys;
    let exchange = issue(&state, &actor, &legacy).await;
    for (index, request) in exchange.requests.iter().enumerate() {
        let collection = collection_for_device_code(db, &request.code);
        let name = collection.name();
        let docs = db.collection::<Document>(name);
        let filter = doc! {"device_code_hmac": hmac_hex(KEY, request.code.as_bytes())};
        if index == 1 {
            docs.update_one(
                filter.clone(),
                doc! {
                    "$set": {"expires_at": bson::DateTime::from_millis(0)},
                    "$unset": {"purge_at": ""},
                },
            )
            .await
            .unwrap();
        } else if request.grant {
            docs.update_one(
                filter.clone(),
                doc! {"$set": {"mfa_policy_revision": "withdrawn"}},
            )
            .await
            .unwrap();
        }
        let source = docs.find_one(filter.clone()).await.unwrap().unwrap();
        match index {
            0 => {
                assert!(matches!(
                    poll_and_prepare(db, keys, KEY, &request.code)
                        .await
                        .unwrap(),
                    PollClaim::Account(_)
                ));
            }
            1 => {
                assert!(record::migrate(db, keys, name, &source).await.unwrap());
                let retained = docs.find_one(filter.clone()).await.unwrap().unwrap();
                assert_eq!(source.get("expires_at"), retained.get("expires_at"));
                assert!(!retained.contains_key("purge_at"));
                assert!(matches!(
                    poll_and_prepare(db, keys, KEY, &request.code)
                        .await
                        .unwrap(),
                    PollClaim::Expired
                ));
            }
            _ => {
                assert!(record::migrate(db, keys, name, &source).await.unwrap());
                app_policy::deny_undelivered(db, name, source.get_str("_id").unwrap(), "withdrawn")
                    .await
                    .unwrap();
                let child = source
                    .get_document("agent_key_grant")
                    .unwrap()
                    .get_str("credential_id")
                    .unwrap();
                assert!(
                    db.collection::<Document>(api_key_credential::COLLECTION_NAME)
                        .find_one(doc! {"_id": child, "is_active": false})
                        .await
                        .unwrap()
                        .is_some()
                );
            }
        }
        assert!(!record::migrate(db, keys, name, &source).await.unwrap());
        let terminal = docs.find_one(filter).await.unwrap().unwrap();
        assert_eq!(
            terminal.get_str("status").unwrap(),
            ["delivered", "expired", "denied"][index]
        );
        assert!(!terminal.contains_key("delivery_context_version"));
        record::verify(keys, name, &terminal).await.unwrap();
    }
}

#[tokio::test]
async fn exact_bson_and_primary_rotation_preserve_each_delivery() {
    let (state, actor) = fixture("device_storage_rotation").await;
    let db = &state.db;
    let mut compatible = crate::test_utils::test_encryption_keys();
    compatible.set_contextual_record_writes(false);
    let exchange = issue(&state, &actor, &compatible).await;
    let mut config = state.config.clone();
    config.encryption_key_previous = config.encryption_key.replace("22".repeat(32));
    config.allow_legacy_record_reads = false;
    let rotated = EncryptionKeys::from_config(&config);
    config.encryption_key_previous = None;
    let primary = EncryptionKeys::from_config(&config);
    for request in &exchange.requests {
        let typed = collection_for_device_code(db, &request.code);
        let name = typed.name();
        let docs = db.collection::<Document>(name);
        let filter = doc! {"device_code_hmac": hmac_hex(KEY, request.code.as_bytes())};
        let first = docs.find_one(filter.clone()).await.unwrap().unwrap();
        let (path, bytes) = if request.grant {
            (
                record::AGENT,
                first
                    .get_document("agent_key_grant")
                    .unwrap()
                    .get_binary_generic("delivery_credential_encrypted")
                    .unwrap(),
            )
        } else {
            (
                record::ACCESS,
                first.get_binary_generic(record::ACCESS).unwrap(),
            )
        };
        let array = bytes
            .iter()
            .map(|&b| Bson::Int32(b.into()))
            .collect::<Vec<_>>();
        docs.update_one(filter.clone(), doc! {"$set": {path: array}})
            .await
            .unwrap();
        assert!(
            !record::migrate(db, &state.encryption_keys, name, &first)
                .await
                .unwrap()
        );
        let current = docs.find_one(filter.clone()).await.unwrap().unwrap();
        assert!(
            record::migrate(db, &state.encryption_keys, name, &current)
                .await
                .unwrap()
        );
        let sealed = docs.find_one(filter.clone()).await.unwrap().unwrap();
        assert!(record::migrate(db, &primary, name, &sealed).await.is_err());
        assert!(docs.find_one(filter.clone()).await.unwrap().unwrap() == sealed);
        assert!(record::migrate(db, &rotated, name, &sealed).await.unwrap());
        record::verify(
            &primary,
            name,
            &docs.find_one(filter).await.unwrap().unwrap(),
        )
        .await
        .unwrap();
    }
    read(db, &primary, &exchange, false).await;
}

#[tokio::test]
async fn missing_child_issuance_keeps_approved_grant_for_review() {
    let (state, actor) = fixture("device_missing_child").await;
    let db = &state.db;
    let mut legacy = crate::test_utils::test_encryption_keys();
    legacy.set_contextual_record_writes(false);
    let exchange = issue(&state, &actor, &legacy).await;
    let request = exchange.requests.iter().find(|r| r.grant).unwrap();
    let docs = db.collection::<Document>(V2_COLLECTION_NAME);
    let filter = doc! {"device_code_hmac": hmac_hex(KEY, request.code.as_bytes())};
    let before = docs.find_one(filter.clone()).await.unwrap().unwrap();
    let child = before
        .get_document("agent_key_grant")
        .unwrap()
        .get_str("credential_id")
        .unwrap();
    db.collection::<Document>(api_key_credential::COLLECTION_NAME)
        .delete_one(doc! {"_id": child})
        .await
        .unwrap();
    let error = record::migrate(db, &state.encryption_keys, V2_COLLECTION_NAME, &before)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(reason) if reason == record::ISSUANCE_MISSING));
    assert!(docs.find_one(filter).await.unwrap().unwrap() == before);
}
