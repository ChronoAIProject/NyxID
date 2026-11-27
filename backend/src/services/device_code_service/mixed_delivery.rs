//! This fixture drives both revisions' approval and signed poll entry points.
use super::*;
use crate::{
    crypto::{aes::EncryptionKeys, token::hash_token},
    models::{
        api_key,
        device_code::COLLECTION_NAME,
        node,
        org_membership::{self, OrgRole},
        user::{self, UserType},
    },
    test_utils::{
        connect_transaction_test_database, test_encryption_keys, test_membership, test_user,
    },
};
use bson::{Document, doc};
use ed25519_dalek::SigningKey;
use futures::TryStreamExt;
use mongodb::Database;
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
struct Request {
    code: String,
    org: bool,
    key_hash: String,
    refresh_hash: String,
    node_id: String,
    key_expiry: Option<String>,
}

impl Request {
    fn filter(&self) -> Document {
        doc! {"device_code_hash": hash_token(&self.code)}
    }

    fn signed(&self) -> DeviceCodePollInput {
        let timestamp = Utc::now().timestamp();
        DeviceCodePollInput {
            device_code: self.code.clone(),
            timestamp,
            signature: tests_support::sign_poll(&self.code, timestamp, &signer(self.org)),
        }
    }
}

fn signer(org: bool) -> SigningKey {
    SigningKey::from_bytes(&[if org { 82 } else { 81 }; 32])
}

async fn issue(db: &Database, keys: &EncryptionKeys) -> Vec<Request> {
    let actor = Uuid::new_v4().to_string();
    let org = Uuid::new_v4().to_string();
    db.collection(user::COLLECTION_NAME)
        .insert_many([
            test_user(&actor, UserType::Person),
            test_user(&org, UserType::Org),
        ])
        .await
        .unwrap();
    db.collection(org_membership::COLLECTION_NAME)
        .insert_one(test_membership(&org, &actor, OrgRole::Admin, None))
        .await
        .unwrap();
    let mut requests = Vec::new();
    for organization in [false, true] {
        let pending = initiate(
            db,
            DeviceCodeInitiateInput {
                device_pubkey: signer(organization).verifying_key().to_bytes(),
                hw_id: "unit-test-node".into(),
                suggested_label: None,
                frontend_url: "https://example.test".into(),
            },
        )
        .await
        .unwrap();
        let approval = approve(
            db,
            keys,
            &actor,
            DeviceCodeApproveInput {
                user_code: pending.user_code,
                org_id: organization.then(|| org.clone()),
                label: None,
                default_services: None,
                allow_auto_connected_services: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            approval.owner_user_id,
            if organization { &org } else { &actor }.as_str()
        );
        let key = db
            .collection::<Document>(api_key::COLLECTION_NAME)
            .find_one(doc! {"_id": &approval.api_key_id})
            .await
            .unwrap()
            .unwrap();
        let row = db
            .collection::<DeviceCode>(COLLECTION_NAME)
            .find_one(doc! {"device_code_hash": hash_token(&pending.device_code)})
            .await
            .unwrap()
            .unwrap();
        requests.push(Request {
            code: pending.device_code,
            org: organization,
            key_hash: key.get_str("key_hash").unwrap().to_owned(),
            refresh_hash: row.refresh_token_hash.unwrap(),
            node_id: approval.node_id,
            key_expiry: approval.api_key_expires_at,
        });
    }
    requests
}

#[derive(Serialize, Deserialize)]
struct Snapshot {
    requests: Vec<Request>,
    rows: std::collections::BTreeMap<String, Vec<Document>>,
}

async fn save(db: &Database, file: &Path, requests: Vec<Request>) {
    let mut rows = std::collections::BTreeMap::new();
    for name in [
        COLLECTION_NAME,
        api_key::COLLECTION_NAME,
        node::COLLECTION_NAME,
    ] {
        rows.insert(
            name.to_owned(),
            db.collection::<Document>(name)
                .find(doc! {})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap(),
        );
    }
    std::fs::write(file, bson::to_vec(&Snapshot { requests, rows }).unwrap()).unwrap();
}

async fn restore(db: &Database, file: &Path) -> Vec<Request> {
    let data: Snapshot = bson::from_slice(&std::fs::read(file).unwrap()).unwrap();
    for (name, rows) in data.rows {
        db.collection::<Document>(&name)
            .insert_many(rows)
            .await
            .unwrap();
    }
    data.requests
}

async fn inspect(db: &Database, keys: &EncryptionKeys, requests: &[Request], step: &str) {
    let docs = db.collection::<Document>(COLLECTION_NAME);
    for request in requests {
        let result = poll(db, keys, request.signed()).await;
        if step == "reject" || step == "legacy-claim-error" {
            assert!(result.is_err());
        } else {
            let DeviceCodePoll::Approved {
                api_key,
                refresh_token,
                node_id,
                api_key_expires_at,
                expires_in,
            } = result.unwrap()
            else {
                panic!("expected approved delivery")
            };
            assert!(
                hash_token(&api_key) == request.key_hash
                    && hash_token(&refresh_token) == request.refresh_hash
            );
            assert_eq!(node_id, request.node_id);
            assert_eq!(api_key_expires_at, request.key_expiry);
            assert_eq!(expires_in, DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS);
        }
        let row = docs.find_one(request.filter()).await.unwrap().unwrap();
        if step == "reject" {
            assert_eq!(row.get_str("status"), Ok("approved"));
            assert!(
                row.contains_key("delivery_api_key_encrypted")
                    && row.contains_key("delivery_refresh_token_encrypted")
            );
        } else {
            assert_eq!(row.get_str("status"), Ok("delivered"));
            assert!(
                !row.contains_key("delivery_api_key_encrypted")
                    && !row.contains_key("delivery_refresh_token_encrypted")
            );
            if step != "legacy-claim-error" {
                assert!(!row.contains_key("delivery_context_version"));
            }
            assert!(matches!(
                poll(db, keys, request.signed()).await,
                Err(AppError::DeviceCodeAlreadyDelivered)
            ));
        }
    }
}

#[tokio::test]
async fn mixed_personal_and_org_signed_delivery() {
    let db = connect_transaction_test_database("mixed_node_device").await;
    let step = std::env::var("NYXID_RECORD_FIXTURE_STEP").unwrap_or_else(|_| "roundtrip".into());
    let file = std::env::var_os("NYXID_RECORD_FIXTURE_FILE");
    let mut keys = test_encryption_keys();
    keys.set_contextual_record_writes(
        std::env::var("CONTEXTUAL_RECORD_WRITES").as_deref() == Ok("true"),
    );
    keys.set_allow_legacy_record_reads(
        std::env::var("ALLOW_LEGACY_RECORD_READS").as_deref() != Ok("false"),
    );
    match step.as_str() {
        "write" | "roundtrip" => {
            let requests = issue(&db, &keys).await;
            if let Some(file) = file {
                save(&db, file.as_ref(), requests).await;
            } else {
                inspect(&db, &keys, &requests, "read").await;
            }
        }
        "legacy-claim-error-save" => {
            let p = file.unwrap();
            let requests = restore(&db, p.as_ref()).await;
            inspect(&db, &keys, &requests, "legacy-claim-error").await;
            save(&db, p.as_ref(), requests).await;
        }
        "read" | "reject" | "legacy-claim-error" => {
            let requests = restore(&db, file.unwrap().as_ref()).await;
            inspect(&db, &keys, &requests, &step).await;
        }
        _ => panic!("unknown fixture step"),
    }
}

// Candidate storage tests
#[tokio::test]
async fn migrate_signed_deliveries_from_the_base_writer() {
    let db = connect_transaction_test_database("migrate_signed_device").await;
    let keys = test_encryption_keys();
    let file = std::env::var_os("NYXID_RECORD_FIXTURE_FILE");
    let requests = if let Some(p) = &file {
        restore(&db, p.as_ref()).await
    } else {
        let mut legacy = test_encryption_keys();
        legacy.set_contextual_record_writes(false);
        issue(&db, &legacy).await
    };
    let docs = db.collection::<Document>(COLLECTION_NAME);
    for request in &requests {
        let before = docs.find_one(request.filter()).await.unwrap().unwrap();
        assert!(!before.contains_key(record::VERSION));
        assert!(record::verify(&keys, &before).await.is_err());
        assert!(record::migrate(&db, &keys, &before).await.unwrap());
        assert!(!record::migrate(&db, &keys, &before).await.unwrap());
        let after = docs.find_one(request.filter()).await.unwrap().unwrap();
        record::verify(&keys, &after).await.unwrap();
        assert!(!record::migrate(&db, &keys, &after).await.unwrap());
        for field in [
            "device_code_hash",
            "device_pubkey",
            "approved_org_id",
            "approved_by_user_id",
            "issued_api_key_id",
            "issued_node_id",
            "expires_at",
            "refresh_token_hash",
        ] {
            assert!(after.get(field) == before.get(field));
        }
        for field in [
            "_id",
            "device_code_hash",
            "approved_by_user_id",
            "approved_org_id",
            "issued_api_key_id",
            "issued_node_id",
        ] {
            let mut swapped = after.clone();
            swapped.insert(field, "other");
            assert!(record::verify(&keys, &swapped).await.is_err(), "{field}");
        }
        let mut signer_swap = after.clone();
        signer_swap.insert(
            "device_pubkey",
            bson::to_bson(&signer(!request.org).verifying_key().to_bytes()).unwrap(),
        );
        assert!(record::verify(&keys, &signer_swap).await.is_err());
        let mut swapped = after.clone();
        swapped.insert(record::API_KEY, after.get(record::REFRESH).unwrap().clone());
        docs.replace_one(request.filter(), &swapped).await.unwrap();
        assert!(poll(&db, &keys, request.signed()).await.is_err());
        assert!(docs.find_one(request.filter()).await.unwrap().unwrap() == swapped);
        docs.replace_one(request.filter(), after).await.unwrap();
    }
    if let Some(file) = file {
        save(&db, file.as_ref(), requests).await;
    } else {
        inspect(&db, &keys, &requests, "read").await;
    }
}

#[tokio::test]
async fn legacy_terminal_marker_clears_without_reissuing() {
    let db = connect_transaction_test_database("node_device_terminal").await;
    let keys = test_encryption_keys();
    let docs = db.collection::<Document>(COLLECTION_NAME);
    let requests = if let Some(p) = std::env::var_os("NYXID_RECORD_FIXTURE_FILE") {
        restore(&db, p.as_ref()).await
    } else {
        let requests = issue(&db, &keys).await;
        for request in &requests {
            docs.update_one(
                request.filter(),
                doc! {
                    "$set": {"status":"delivered"},
                    "$unset": {record::API_KEY:"", record::REFRESH:""},
                },
            )
            .await
            .unwrap();
        }
        requests
    };
    for request in requests {
        let before = docs.find_one(request.filter()).await.unwrap().unwrap();
        assert_eq!(before.get_i32(record::VERSION), Ok(3));
        assert_eq!(before.get_str("status"), Ok("delivered"));
        assert!(record::verify(&keys, &before).await.is_err());
        assert!(record::migrate(&db, &keys, &before).await.unwrap());
        let after = docs.find_one(request.filter()).await.unwrap().unwrap();
        let mut expected = before;
        expected.remove(record::VERSION);
        assert!(after == expected);
        record::verify(&keys, &after).await.unwrap();
        assert!(matches!(
            poll(&db, &keys, request.signed()).await,
            Err(AppError::DeviceCodeAlreadyDelivered)
        ));
    }
}

#[tokio::test]
async fn signed_claim_expiry_and_ttl_fence_storage() {
    let db = connect_transaction_test_database("node_device_claim_expiry").await;
    let mut legacy = test_encryption_keys();
    legacy.set_contextual_record_writes(false);
    let keys = test_encryption_keys();
    let requests = issue(&db, &legacy).await;
    let docs = db.collection::<Document>(COLLECTION_NAME);
    for request in requests {
        let before = docs.find_one(request.filter()).await.unwrap().unwrap();
        if request.org {
            docs.update_one(
                request.filter(),
                doc! {"$set":{"expires_at": bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
            assert!(matches!(
                poll(&db, &keys, request.signed()).await,
                Err(AppError::DeviceCodeExpired)
            ));
            let expired = docs.find_one(request.filter()).await.unwrap().unwrap();
            assert_eq!(expired.get_str("status"), Ok("expired"));
            assert!(!record::migrate(&db, &keys, &before).await.unwrap());
            assert!(record::migrate(&db, &keys, &expired).await.unwrap());
            let sealed = docs.find_one(request.filter()).await.unwrap().unwrap();
            record::verify(&keys, &sealed).await.unwrap();
            assert_eq!(sealed.get("expires_at"), expired.get("expires_at"));
            assert_eq!(sealed.get_str("status"), Ok("expired"));
            docs.delete_one(request.filter()).await.unwrap();
            assert!(!record::migrate(&db, &keys, &expired).await.unwrap());
            assert!(docs.find_one(request.filter()).await.unwrap().is_none());
        } else {
            assert!(record::migrate(&db, &keys, &before).await.unwrap());
            let wrong = DeviceCodePollInput {
                signature: [0; 64],
                ..request.signed()
            };
            assert!(matches!(
                poll(&db, &keys, wrong).await,
                Err(AppError::DevicePollSignatureInvalid(_))
            ));
            let sealed = docs.find_one(request.filter()).await.unwrap().unwrap();
            record::verify(&keys, &sealed).await.unwrap();
            let (a, b) = tokio::join!(
                poll(&db, &keys, request.signed()),
                poll(&db, &keys, request.signed())
            );
            let results = [a, b];
            assert_eq!(
                results
                    .iter()
                    .filter(|v| matches!(v, Ok(DeviceCodePoll::Approved { .. })))
                    .count(),
                1
            );
            assert_eq!(
                results
                    .iter()
                    .filter(|v| matches!(v, Err(AppError::DeviceCodeAlreadyDelivered)))
                    .count(),
                1
            );
            assert!(!record::migrate(&db, &keys, &before).await.unwrap());
            let after = docs.find_one(request.filter()).await.unwrap().unwrap();
            assert!(
                !after.contains_key(record::API_KEY)
                    && !after.contains_key(record::REFRESH)
                    && !after.contains_key(record::VERSION)
            );
            record::verify(&keys, &after).await.unwrap();
        }
    }
}

#[tokio::test]
async fn migration_authenticates_issuance_representation_and_primary() {
    let db = connect_transaction_test_database("node_device_evidence").await;
    let mut legacy = test_encryption_keys();
    legacy.set_contextual_record_writes(false);
    let keys = test_encryption_keys();
    let requests = issue(&db, &legacy).await;
    let docs = db.collection::<Document>(COLLECTION_NAME);
    for request in requests {
        let before = docs.find_one(request.filter()).await.unwrap().unwrap();
        for field in [
            record::API_KEY,
            record::REFRESH,
            "approved_org_id",
            "issued_node_id",
            "refresh_token_hash",
        ] {
            let mut inconsistent = before.clone();
            match field {
                record::API_KEY | record::REFRESH => {
                    let other = if field == record::API_KEY {
                        record::REFRESH
                    } else {
                        record::API_KEY
                    };
                    inconsistent.insert(field, before.get(other).unwrap().clone());
                }
                _ => {
                    inconsistent.insert(field, "different");
                }
            }
            assert!(
                record::migrate(&db, &keys, &inconsistent).await.is_err(),
                "{field}"
            );
        }
        if request.org {
            db.collection::<Document>(api_key::COLLECTION_NAME)
                .delete_one(doc! {"_id":before.get_str("issued_api_key_id").unwrap()})
                .await
                .unwrap();
            let Err(AppError::Conflict(reason)) = record::migrate(&db, &keys, &before).await else {
                panic!("missing issuance witness")
            };
            assert_eq!(reason, record::ISSUANCE_MISSING);
            assert!(docs.find_one(request.filter()).await.unwrap().unwrap() == before);
        } else {
            let array = before
                .get_binary_generic(record::API_KEY)
                .unwrap()
                .iter()
                .map(|b| bson::Bson::Int32((*b).into()))
                .collect::<Vec<_>>();
            docs.update_one(request.filter(), doc! {"$set":{record::API_KEY:array}})
                .await
                .unwrap();
            assert!(!record::migrate(&db, &keys, &before).await.unwrap());
            let observed = docs.find_one(request.filter()).await.unwrap().unwrap();
            assert!(record::migrate(&db, &keys, &observed).await.unwrap());
            let sealed = docs.find_one(request.filter()).await.unwrap().unwrap();
            let mut config = crate::test_utils::test_app_config();
            config.encryption_key_previous = config.encryption_key.replace("22".repeat(32));
            config.allow_legacy_record_reads = false;
            let rotated = EncryptionKeys::from_config(&config);
            config.encryption_key_previous = None;
            let primary = EncryptionKeys::from_config(&config);
            assert!(record::migrate(&db, &primary, &sealed).await.is_err());
            assert!(record::migrate(&db, &rotated, &sealed).await.unwrap());
            let after = docs.find_one(request.filter()).await.unwrap().unwrap();
            record::verify(&primary, &after).await.unwrap();
            inspect(&db, &primary, &[request], "read").await;
        }
    }
}

use crate::crypto::{
    key_provider::{KeyProvider, WrappedKey},
    local_key_provider::LocalKeyProvider,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use zeroize::Zeroizing;
#[derive(Debug)]
struct PausedProvider {
    local: LocalKeyProvider,
    pause: AtomicBool,
    entered: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}
#[async_trait::async_trait]
impl KeyProvider for PausedProvider {
    async fn wrap_dek(&self, key: &[u8]) -> AppResult<WrappedKey> {
        self.local.wrap_dek(key).await
    }
    async fn unwrap_dek(&self, key: &WrappedKey) -> AppResult<Zeroizing<Vec<u8>>> {
        if self.pause.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.resume.notified().await;
        }
        self.local.unwrap_dek(key).await
    }
    fn current_key_id(&self) -> u8 {
        self.local.current_key_id()
    }
    fn has_key_id(&self, id: u8) -> bool {
        self.local.has_key_id(id)
    }
    fn has_previous_key(&self) -> bool {
        false
    }
}

#[tokio::test]
async fn signed_poll_rechecks_the_winning_rewrap_or_expiry() {
    let db = connect_transaction_test_database("node_device_interleave").await;
    let provider = Arc::new(PausedProvider {
        local: LocalKeyProvider::new([0x11; 32], None),
        pause: AtomicBool::new(false),
        entered: Default::default(),
        resume: Default::default(),
    });
    let keys = EncryptionKeys::with_provider(provider.clone());
    let mut legacy = test_encryption_keys();
    legacy.set_contextual_record_writes(false);
    let docs = db.collection::<Document>(COLLECTION_NAME);
    for request in issue(&db, &legacy).await {
        provider.pause.store(true, Ordering::SeqCst);
        let change = async {
            provider.entered.notified().await;
            let before = docs.find_one(request.filter()).await.unwrap().unwrap();
            if request.org {
                docs.update_one(
                    request.filter(),
                    doc! {"$set":{"expires_at":bson::DateTime::from_millis(0)}},
                )
                .await
                .unwrap();
            } else {
                assert!(
                    record::migrate(&db, &test_encryption_keys(), &before)
                        .await
                        .unwrap()
                );
            }
            provider.resume.notify_one();
        };
        let (result, ()) = tokio::join!(poll(&db, &keys, request.signed()), change);
        let after = docs.find_one(request.filter()).await.unwrap().unwrap();
        if request.org {
            assert!(matches!(result, Err(AppError::DeviceCodeExpired)));
            assert_eq!(after.get_str("status"), Ok("expired"));
            assert!(after.contains_key(record::API_KEY) && after.contains_key(record::REFRESH));
        } else {
            let Ok(DeviceCodePoll::Approved {
                api_key,
                refresh_token,
                ..
            }) = result
            else {
                panic!("rewrap interrupted signed delivery")
            };
            assert!(
                hash_token(&api_key) == request.key_hash
                    && hash_token(&refresh_token) == request.refresh_hash
            );
            assert_eq!(after.get_str("status"), Ok("delivered"));
            assert!(
                !after.contains_key(record::API_KEY)
                    && !after.contains_key(record::REFRESH)
                    && !after.contains_key(record::VERSION)
            );
        }
    }
}
