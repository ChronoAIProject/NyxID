use super::*;
use crate::models::{assistant_cma_creation, assistant_cma_sandbox};
use crate::services::assistant_cma_sandbox_service::prepare_creation_request;

const FIELD: &str = "request_ciphertext";
const VERSION: &str = "request_context_version";
const WIRE: &[u8] = br#"{ "name":"one", "name":"two\u0021" }"#;

// Unmarked v2 fixtures from the server tests at 846b09d4d.
const LEGACY_REQUEST: &str = concat!(
    "0202003c91a13b8af61931e860740dae6af04bda0b031c78a79267cff72a495e0e9ddb2dcaec624d",
    "0b0d4cd35bd62d9a1039134a447583f3652946e755510a394b850d2c2d295666b54506d18542b893",
    "df4383857ef61bc4baa554d506163615a6980f0e1282588d08353dde3e413de9f0491f43ed24da3d",
    "319505b0bf1a1819",
);

#[tokio::test]
async fn cma_requests_authenticate_both_contexts_and_preserve_exact_source_winners() {
    let db = crate::test_utils::connect_transaction_test_database("cma_request_records").await;
    let mut config = crate::test_utils::test_app_config();
    let old = EncryptionKeys::from_config(&config);
    config.encryption_key_previous = config.encryption_key.replace("33".repeat(32));
    let keys = EncryptionKeys::from_config(&config);
    let hash = prepare_creation_request(WIRE).unwrap().hash;
    for entry in REGISTRY
        .iter()
        .filter(|e| matches!(e.adapter, Adapter::CmaRequest))
    {
        let rows = db.collection::<Document>(entry.collection);
        let id = doc! {"_id": "one"};
        let bytes = hex::decode(LEGACY_REQUEST).unwrap();
        let original = doc! {
            "_id": "one",
            "user_id": "owner",
            "request_hash": &hash,
            "submission_state": "unknown",
            "sibling": true,
            FIELD: bson::to_bson(&bytes).unwrap(),
        };
        rows.insert_one(&original).await.unwrap();
        let other = if entry.collection == assistant_cma_creation::COLLECTION_NAME {
            assistant_cma_sandbox::COLLECTION_NAME
        } else {
            assistant_cma_creation::COLLECTION_NAME
        };
        for change in [
            doc! {"generation": 2},
            doc! {"submission_generation": 1},
            doc! {"revision": 1},
            doc! {"operation_id": "received"},
            doc! {"request_hash": "successor"},
            doc! {VERSION: Bson::Null},
            doc! {FIELD: Bson::Null},
            doc! {FIELD: bson::Binary {subtype: bson::spec::BinarySubtype::Generic, bytes: bytes.clone()}},
        ] {
            rows.update_one(id.clone(), doc! {"$set": change})
                .await
                .unwrap();
            let winner = rows.find_one(id.clone()).await.unwrap().unwrap();
            assert!(!migrate(entry, &db, &old, &original).await.unwrap());
            assert!(rows.find_one(id.clone()).await.unwrap().unwrap() == winner);
            rows.replace_one(id.clone(), &original).await.unwrap();
        }
        assert!(migrate(entry, &db, &old, &original).await.unwrap());
        let previous_primary = rows.find_one(id.clone()).await.unwrap().unwrap();
        assert!(verify(entry, &db, &keys, &previous_primary).await.is_err());
        assert!(migrate(entry, &db, &keys, &previous_primary).await.unwrap());
        let current = rows.find_one(id.clone()).await.unwrap().unwrap();
        super::swaps::assert_registry_swaps(entry, &db, &keys, &current, "user_id").await;
        assert!(!migrate(entry, &db, &keys, &current).await.unwrap());
        let opened = crate::crypto::record_field::decrypt(
            &keys,
            entry.collection,
            "one",
            "owner",
            FIELD,
            Some(3),
            current.get_binary_generic(FIELD).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(opened.as_slice(), WIRE);
        let mut preserved = current.clone();
        preserved.remove(VERSION);
        preserved.insert(FIELD, original.get(FIELD).unwrap().clone());
        assert!(preserved == original);
        let swapped =
            crate::crypto::record_field::encrypt(&keys, other, "one", "owner", FIELD, WIRE)
                .await
                .unwrap();
        let wrong_purpose = crate::crypto::record_field::encrypt(
            &keys,
            entry.collection,
            "one",
            "owner",
            "other",
            WIRE,
        )
        .await
        .unwrap();
        for delta in [
            doc! {FIELD: bson::to_bson(&swapped).unwrap()},
            doc! {FIELD: bson::to_bson(&wrong_purpose).unwrap()},
            doc! {"request_hash": prepare_creation_request(br#"{"name":"two!"}"#).unwrap().hash},
            doc! {VERSION: 259},
            doc! {VERSION: -1},
            doc! {VERSION: "3"},
            doc! {VERSION: Bson::Null},
            doc! {FIELD: bson::to_bson(&bytes).unwrap()},
            doc! {FIELD: [256]},
            doc! {FIELD: []},
            doc! {FIELD: "invalid"},
            doc! {FIELD: Bson::Null},
        ] {
            let mut bad = current.clone();
            bad.extend(delta);
            assert!(verify(entry, &db, &keys, &bad).await.is_err());
            assert!(migrate(entry, &db, &keys, &bad).await.is_err());
        }
        let mut missing = original.clone();
        missing.remove(FIELD);
        assert!(verify(entry, &db, &keys, &missing).await.is_err());
        missing.extend(doc! {
            "admission_state": "retired",
            "submission_state": "confirmed_not_created",
            "occupies_account_slot": false,
        });
        verify(entry, &db, &keys, &missing).await.unwrap();
        if entry.collection == assistant_cma_sandbox::COLLECTION_NAME {
            let external = doc! {"_id":"ref","user_id":"owner","source":"discovered"};
            verify(entry, &db, &keys, &external).await.unwrap();
        }
    }
}

#[tokio::test]
async fn cma_generation_resumes_after_rewrap_and_terminal_cleanup() {
    let db = crate::test_utils::connect_transaction_test_database("cma_request_walk").await;
    let keys = crate::test_utils::test_encryption_keys();
    let generation = start(&db, &keys, Mode::Migrate, None).await.unwrap();
    let index = REGISTRY
        .iter()
        .position(|e| matches!(e.adapter, Adapter::CmaRequest))
        .unwrap();
    db.collection::<Document>(RUNS)
        .update_one(doc! {"_id": RUN_ID}, doc! {"$set": {"entry": index as i32}})
        .await
        .unwrap();
    for entry in &REGISTRY[index..index + 2] {
        let rows = db.collection::<Document>(entry.collection);
        let retained = doc! {
            "_id": "retained",
            "user_id": "owner",
            "submission_state": "unknown",
            "request_hash": prepare_creation_request(WIRE).unwrap().hash,
            FIELD: bson::to_bson(&hex::decode(LEGACY_REQUEST).unwrap()).unwrap(),
        };
        let cleared = doc! {
            "_id": "cleared",
            "user_id": "owner",
            "submission_state": "confirmed_not_created",
            "admission_state": "retired",
            "occupies_account_slot": false,
            FIELD: Bson::Null,
            VERSION: Bson::Null,
        };
        let mut damaged = retained.clone();
        damaged.extend(doc! {"_id":"blocked",VERSION:259});
        rows.insert_many([&retained, &cleared, &damaged])
            .await
            .unwrap();
        assert!(migrate(entry, &db, &keys, &retained).await.unwrap());
        assert!(advance(&db, &keys, "stale").await.is_err());
        Box::pin(advance(&db, &keys, &generation)).await.unwrap();
        let result = status(&db, &keys).await.unwrap().unwrap();
        let outcome = result
            .field_outcomes
            .iter()
            .find(|x| x.collection == entry.collection)
            .unwrap();
        assert_eq!(
            (
                outcome.authenticated_current,
                outcome.no_material_observed,
                outcome.verification_blocked
            ),
            (1, 1, 1)
        );
        assert_eq!(result.rewritten, 0);
    }
}
