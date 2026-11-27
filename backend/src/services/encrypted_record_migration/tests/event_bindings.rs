use super::*;
use crate::{
    crypto::{record_field::RecordContext, token::hash_token},
    models::api_key,
};

const PROVIDER: &str = "provider_key_ciphertext";
const PRODUCER: &str = "producer_secret_ciphertext";

// Unmarked v2 fixtures from the server tests at 846b09d4d.
const LEGACY_PROVIDER_ONE: &str = concat!(
    "0202003c43d890a74c66728a70779dc680130c7f03d2176c959b3637945ef578fec0f5916e786289",
    "5a594e75d590d79cb72f4b6643ac3597b04ffe4c05bacafbae0cb0436582b1c147fb15e05c3451cd",
    "f359ed6b6581031616e30355dcaac6b27c3e0287d76a8b5947aa06b63217be",
);

const LEGACY_PROVIDER_SAME_OWNER: &str = concat!(
    "0202003c0351090c44ea389eabd4e3db937c8ed1cb48e404b551f956029d86bf21419583ec80bc32",
    "9796fc1349ed99382a681c9fc4713ef51d4b0469f4889e833b1aa33c39a2669c6395ce4d37dd3a9b",
    "2975b0729a60996b85c20889d2739fdb93ad023d757a9b5ae39c04a6c854a4a6112c1a3ef3c1",
);

const LEGACY_PROVIDER_OTHER_OWNER: &str = concat!(
    "0202003cc74ad9d3e86c8c8b63ba185bf2cf67beade3ec830a0d1e6f7c0829876521821c54031a03",
    "f98efa0af9cab5c3d306bc4124b260000af370e059cae05fcabcc3a7d516dae2223b04454de69de7",
    "ddd07889dafba08b56ab0d9c3bb30e68fa486b408c144076f70b15cdce1568d54184ff1a42fa45",
);

const LEGACY_PROVIDER_READY: &str = concat!(
    "0202003ce0e5c9ba90e688a6cf3477d4c5f209f2cb8055d5796aaf15fe72fb4a68a71a4e0306d628",
    "27cdf1734f21a0c3cacc1adeb117560181ba6321bb956387438b3584bacd634b8f54917fc87a893c",
    "997bf69a07e69141247fa8e475aba94e04996b9cdf83229e9729a3df0e36cd8884",
);

const LEGACY_PRODUCER: &str = concat!(
    "0202003c6493e85fe6b5524e416aec90b021db3794ed84eae990083f4e16d1812e18a55b2699273a",
    "4a68aab73e0866a1ace90c9691a748b68094cb129e4cfb7cd53c59f69d385092ddbf5c41caa9e6eb",
    "2928e72513fde81d88a03bf4121f3343a370408f32e0c72cbc6664d2495f1830bf1a40357a41a216",
    "30025e2462da52f8e314693613abaaec",
);

fn binary(bytes: Vec<u8>) -> Bson {
    bson::Binary {
        subtype: bson::spec::BinarySubtype::Generic,
        bytes,
    }
    .into()
}

async fn legacy(db: &Database, id: &str, owner: &str, provider_ciphertext: &str) -> Document {
    let api_id = format!("managed-{id}");
    let bearer = format!("fixture-api-key-{id}");
    db.collection::<Document>(api_key::COLLECTION_NAME)
        .insert_one(doc! {"_id":&api_id,"user_id":owner,"key_hash":hash_token(&bearer)})
        .await
        .unwrap();
    doc! {"_id":id,"user_id":owner,"agent_id":"specialist","agent_owner_id":"organization",
    "destination_channel_id":"chat","destination_partition":"partition",
    "provider_api_key_id":api_id,"producer_key_id":format!("producer-{id}"),
    PROVIDER:bson::to_bson(&hex::decode(provider_ciphertext).unwrap()).unwrap(),
    PRODUCER:binary(hex::decode(LEGACY_PRODUCER).unwrap()),
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
    let source = legacy(&db, "one", "person", LEGACY_PROVIDER_ONE).await;
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
        // The authenticated bearer is too short for a producer secret.
        doc! {PRODUCER:binary(hex::decode(LEGACY_PROVIDER_ONE).unwrap())},
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

    for (other_id, owner, provider) in [
        ("same-owner", "person", LEGACY_PROVIDER_SAME_OWNER),
        ("other-owner", "other", LEGACY_PROVIDER_OTHER_OWNER),
    ] {
        let other = legacy(&db, other_id, owner, provider).await;
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
    let ready = legacy(&db, "ready", "owner", LEGACY_PROVIDER_READY).await;
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
