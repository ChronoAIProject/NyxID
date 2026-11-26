use super::*;
use crate::{
    crypto::{record_field::RecordContext, token::hash_token},
    models::api_key,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};

const PROVIDER: &str = "provider_key_ciphertext";
const PRODUCER: &str = "producer_secret_ciphertext";

fn binary(bytes: Vec<u8>) -> Bson {
    bson::Binary {
        subtype: bson::spec::BinarySubtype::Generic,
        bytes,
    }
    .into()
}

async fn legacy(db: &Database, keys: &EncryptionKeys, id: &str, owner: &str) -> Document {
    let api_id = format!("managed-{id}");
    let bearer = format!("fixture-api-key-{id}");
    db.collection::<Document>(api_key::COLLECTION_NAME)
        .insert_one(doc! {"_id":&api_id,"user_id":owner,"key_hash":hash_token(&bearer)})
        .await
        .unwrap();
    doc! {"_id":id,"user_id":owner,"agent_id":"specialist","agent_owner_id":"organization",
    "destination_channel_id":"chat","destination_partition":"partition",
    "provider_api_key_id":api_id,"producer_key_id":format!("producer-{id}"),
    PROVIDER:bson::to_bson(&keys.encrypt(bearer.as_bytes()).await.unwrap()).unwrap(),
    PRODUCER:binary(keys.encrypt(STANDARD.encode([7;32]).as_bytes()).await.unwrap()),
    "status":"ready","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),"sibling":"preserved"}
}

#[tokio::test]
async fn event_binding_rewrap_fences_both_purposes_and_checks_the_installed_primary() {
    let db = crate::test_utils::connect_transaction_test_database("event_binding_keys").await;
    let mut config = crate::test_utils::test_app_config();
    let old = EncryptionKeys::from_config(&config);
    config.encryption_key_previous = config.encryption_key.replace("33".repeat(32));
    let current = EncryptionKeys::from_config(&config);
    let entry = REGISTRY
        .iter()
        .find(|e| matches!(e.adapter, Adapter::EventBinding))
        .unwrap();
    let rows = db.collection::<Document>(entry.collection);
    let source = legacy(&db, &old, "one", "person").await;
    let id = doc! {"_id":"one"};
    rows.insert_one(&source).await.unwrap();
    for delta in [
        doc! {"status":"revoked"},
        doc! {"provider_binding_id":"bound"},
        doc! {"user_id":"organization"},
        doc! {"agent_owner_id":"person"},
        doc! {"provider_api_key_id":"replacement"},
        doc! {"producer_key_id":"rotated"},
        doc! {"secret_context_versions":{}},
        doc! {PROVIDER:binary(bson::from_bson(source.get(PROVIDER).unwrap().clone()).unwrap())},
        doc! {PRODUCER:Bson::Null},
    ] {
        rows.update_one(id.clone(), doc! {"$set":delta})
            .await
            .unwrap();
        let winner = rows.find_one(id.clone()).await.unwrap().unwrap();
        assert!(!migrate(entry, &db, &old, &source).await.unwrap());
        assert!(rows.find_one(id.clone()).await.unwrap().unwrap() == winner);
        rows.replace_one(id.clone(), &source).await.unwrap();
    }
    for delta in [
        doc! {"user_id":"organization"},
        doc! {"provider_api_key_id":"unknown"},
        doc! {PROVIDER:source.get(PRODUCER).unwrap().clone()},
        doc! {PRODUCER:binary(old.encrypt(b"incorrect-length").await.unwrap())},
        doc! {PRODUCER:Bson::Null},
        doc! {"secret_context_versions":{PRODUCER:4}},
    ] {
        let mut bad = source.clone();
        bad.extend(delta);
        assert!(migrate(entry, &db, &old, &bad).await.is_err());
    }
    assert!(migrate(entry, &db, &old, &source).await.unwrap());
    let prior = rows.find_one(id.clone()).await.unwrap().unwrap();
    assert!(verify(entry, &db, &current, &prior).await.is_err());
    assert!(migrate(entry, &db, &current, &prior).await.unwrap());
    let winner = rows.find_one(id.clone()).await.unwrap().unwrap();
    super::swaps::assert_registry_swaps(entry, &db, &current, &winner, "user_id").await;
    assert!(!migrate(entry, &db, &current, &winner).await.unwrap());
    assert!(migrate(entry, &db, &old, &winner).await.is_err());
    // A mixed-format source keeps the independently current sibling byte-for-byte.
    let mut mixed = winner.clone();
    mixed.insert(PRODUCER, source.get(PRODUCER).unwrap().clone());
    mixed
        .get_document_mut("secret_context_versions")
        .unwrap()
        .remove(PRODUCER);
    rows.replace_one(id.clone(), mixed.clone()).await.unwrap();
    assert!(migrate(entry, &db, &current, &mixed).await.unwrap());
    let rebound = rows.find_one(id.clone()).await.unwrap().unwrap();
    assert!(rebound.get(PROVIDER).unwrap() == winner.get(PROVIDER).unwrap());
    assert_eq!(rebound.get_str("sibling"), Ok("preserved"));

    for (other_id, owner) in [("same-owner", "person"), ("other-owner", "other")] {
        let other = legacy(&db, &old, other_id, owner).await;
        rows.insert_one(&other).await.unwrap();
        assert!(migrate(entry, &db, &current, &other).await.unwrap());
        let other = rows.find_one(doc! {"_id":other_id}).await.unwrap().unwrap();
        for field in [PROVIDER, PRODUCER] {
            let mut swapped = winner.clone();
            swapped.insert(field, other.get(field).unwrap().clone());
            assert!(verify(entry, &db, &current, &swapped).await.is_err());
            assert!(migrate(entry, &db, &current, &swapped).await.is_err());
        }
    }
    for field in [PROVIDER, PRODUCER] {
        for mismatch in [
            "agent_id",
            "agent_owner_id",
            if field == PROVIDER {
                "provider_api_key_id"
            } else {
                "producer_key_id"
            },
        ] {
            let mut wrong = winner.clone();
            wrong.insert(mismatch, "different");
            assert!(verify(entry, &db, &current, &wrong).await.is_err());
        }
        let key = if field == PROVIDER {
            "managed-one"
        } else {
            "producer-one"
        };
        let path = [field, key, "specialist", "organization"];
        let other = RecordContext::new("nyxbot_channels", "one", "person", &path).unwrap();
        let mut damaged = winner.get_binary_generic(field).unwrap().clone();
        *damaged.last_mut().unwrap() ^= 1;
        for material in [
            binary(current.encrypt_record(b"fixture", &other).await.unwrap()),
            binary(damaged),
            binary(vec![3, 78, 89, 88]),
            binary(vec![]),
            Bson::Null,
            Bson::String("malformed".into()),
        ] {
            let mut bad = winner.clone();
            bad.insert(field, material);
            assert!(verify(entry, &db, &current, &bad).await.is_err());
            assert!(migrate(entry, &db, &current, &bad).await.is_err());
        }
    }
}

#[tokio::test]
async fn event_binding_generation_resumes_with_distinct_field_outcomes_after_terminal_cleanup() {
    let db = crate::test_utils::connect_transaction_test_database("event_binding_generation").await;
    let keys = crate::test_utils::test_encryption_keys();
    let index = REGISTRY
        .iter()
        .position(|e| matches!(e.adapter, Adapter::EventBinding))
        .unwrap();
    let entry = &REGISTRY[index];
    let rows = db.collection::<Document>(entry.collection);
    let ready = legacy(&db, &keys, "ready", "owner").await;
    let mut failed = ready.clone();
    failed.extend(doc! {"_id":"failed","status":"failed",PRODUCER:Bson::Null,PROVIDER:Bson::Null});
    let mut revoked = failed.clone();
    revoked.extend(doc! {"_id":"revoked","status":"revoked","revoked_at":bson::DateTime::now()});
    revoked.remove(PRODUCER);
    revoked.remove(PROVIDER);
    let mut bad = ready.clone();
    bad.extend(doc! {"_id":"incomplete","status":"preparing",PRODUCER:Bson::Null});
    rows.insert_many([&ready, &failed, &revoked, &bad])
        .await
        .unwrap();
    let generation = start(&db, &keys, Mode::Migrate, None).await.unwrap();
    db.collection::<Document>(RUNS)
        .update_one(doc! {"_id":RUN_ID}, doc! {"$set":{"entry":index as i32}})
        .await
        .unwrap();
    assert!(migrate(entry, &db, &keys, &ready).await.unwrap());
    assert!(advance(&db, &keys, "stale").await.is_err());
    Box::pin(advance(&db, &keys, &generation)).await.unwrap();
    let report = status(&db, &keys).await.unwrap().unwrap();
    assert_eq!(
        (report.scanned, report.rewritten, report.unresolved),
        (4, 0, 1)
    );
    for field in [PROVIDER, PRODUCER] {
        let counts = report
            .field_outcomes
            .iter()
            .find(|x| x.collection == entry.collection && x.field == field)
            .unwrap();
        assert_eq!(
            (
                counts.authenticated_current,
                counts.no_material_observed,
                counts.verification_blocked
            ),
            (1, 2, 1)
        );
    }
}
