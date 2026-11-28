use super::*;
use crate::services::oauth_client_webhook_record::{self, CONNECTION, REVOCATION};

use super::swaps::assert_registry_swaps;

// Unmarked v2 fixtures captured with the previous test key (0x22).
const OLD_REVOCATION: &str = concat!(
    "029f003ce37046aeae831ae1d07467b6a392421a6b974887a87a1f7470579fd52ae8a7cd6c4c0604",
    "0eb9464ff1b35db2ce636e18ff0ce32081d37abfd4dedc42afc27fe213fad2fd8ff3c50a18343915",
    "186b8a51d578dd222bb4a75f69deb6d7a4ab68a2504c43c726c94af7cf72",
);

const OLD_CONNECTION: &str = concat!(
    "029f003c212427bcba377c356d7a3fdc1a7a49fec02b03c48dabaf315cabb0853d562ce83e1c0ded",
    "72f4bdea1386134ac22570e88cd28e4bc7638eacc149cb52ac178df2277a4fefba46221bb7a08e04",
    "e7da0390a194bcd9a7290534614e121d3ed03e82e9ce509aa304d45740a2",
);

const OLD_ROTATED_CONNECTION: &str = concat!(
    "029f003ce6fbdb0401fc66f4c2aa67fe9be7502ca95ed9e08799dc238f4dbc374d142135953e0525",
    "c272ebb1d3589ec1bcaccde87c21444ca9d2f6933e3bdb4769afbcd756e15b6d3556ad459c867b80",
    "8662d5ca1a74d6a6aa25870a4634c79517a4d4",
);

// The generation walk used the standard test key (0x11).
const LEGACY_REVOCATION: &str = concat!(
    "0202003c8691b4520fdaa20effc677fb10e7b137ab3eec6f7cdd4490c7fe275b55f4ad9fcf2a606e",
    "20d11613d88ee42af4ad8c01cd68e4583349b7eda2ada230bdd1fadb615117d39c5545fc3c217ff5",
    "08d35123c4a49eb2afc3cad829601583005d",
);

const INVALID_UTF8_CONNECTION: &str = concat!(
    "0202003c8c82e0c78c3b41f49983440d95fd114a9f674d7180c947a93df086ab54016a048db7fe8d",
    "def09c4908cc807384bc6c8c32e534db238076ba1ac66ef4ab30fa89242300b3eeb8e4ceaeb2b939",
    "8f8a23e43d0e9bf33bf0cf955b",
);

fn bytes(value: Vec<u8>) -> Bson {
    Bson::Binary(bson::Binary {
        subtype: bson::spec::BinarySubtype::Generic,
        bytes: value,
    })
}

#[tokio::test]
async fn webhook_rewrap_fences_ownership_rotation_disable_and_removal() {
    let db = crate::test_utils::connect_transaction_test_database("webhook_record").await;
    let entry = REGISTRY
        .iter()
        .find(|e| e.collection == "oauth_clients")
        .unwrap();
    let rows = db.collection::<Document>(entry.collection);
    let mut config = crate::test_utils::test_app_config();
    config.encryption_key = Some("22".repeat(32));
    let old = EncryptionKeys::from_config(&config);
    config.encryption_key_previous = config.encryption_key.replace("11".repeat(32));
    let keys = EncryptionKeys::from_config(&config);
    let mut source = doc! {"_id":"row", "created_by":"owner", "updated_at":bson::DateTime::now(),
    "connection_webhook_enabled":true, "connection_webhook_url":"https://example.test/",
    "connection_webhook_key_id":"key-1", "other_authority": 9,
    REVOCATION: bson::to_bson(&hex::decode(OLD_REVOCATION).unwrap()).unwrap(),
    CONNECTION: bytes(hex::decode(OLD_CONNECTION).unwrap())};
    rows.insert_one(&source).await.unwrap();
    let lookup = doc! {"_id":"row"};
    // The snapshot is from before each competing operation. Even a change to
    // representation alone is an exact-source conflict.
    for update in [
        doc! {"$set":{"created_by":"org"}},
        doc! {"$set":{CONNECTION:bytes(hex::decode(OLD_ROTATED_CONNECTION).unwrap()),"connection_webhook_key_id":"key-2"}},
        doc! {"$set":{"connection_webhook_enabled":false},"$unset":{CONNECTION:""}},
        doc! {"$set":{REVOCATION:bytes(crate::services::record_field_service::stored_bytes(source.get(REVOCATION).unwrap()).unwrap())}},
    ] {
        rows.update_one(lookup.clone(), update).await.unwrap();
        let winner = rows.find_one(lookup.clone()).await.unwrap().unwrap();
        assert!(!migrate(entry, &db, &keys, &source).await.unwrap());
        assert_eq!(
            rows.find_one(lookup.clone()).await.unwrap().unwrap(),
            winner
        );
        rows.replace_one(lookup.clone(), &source).await.unwrap();
    }
    rows.delete_one(lookup.clone()).await.unwrap();
    assert!(!migrate(entry, &db, &keys, &source).await.unwrap());
    assert!(rows.find_one(lookup.clone()).await.unwrap().is_none());

    // Absent, null, reserved and explicitly owned registrations have distinct
    // scopes except for the two encodings of a missing creator.
    for creator in [
        None,
        Some(Bson::Null),
        Some("system".into()),
        Some("dynamic_registration".into()),
        Some("owner".into()),
    ] {
        source.remove("created_by");
        if let Some(value) = creator {
            source.insert("created_by", value);
        }
        rows.insert_one(&source).await.unwrap();
        assert!(verify(entry, &db, &keys, &source).await.is_err());
        assert!(migrate(entry, &db, &old, &source).await.unwrap());
        let old_current = rows.find_one(lookup.clone()).await.unwrap().unwrap();
        assert!(verify(entry, &db, &keys, &old_current).await.is_err());
        assert!(migrate(entry, &db, &keys, &old_current).await.unwrap());
        let current = rows.find_one(lookup.clone()).await.unwrap().unwrap();
        assert_registry_swaps(entry, &db, &keys, &current, "created_by").await;
        assert!(!migrate(entry, &db, &keys, &current).await.unwrap());
        for field in [
            "updated_at",
            "created_by",
            "connection_webhook_key_id",
            "other_authority",
        ] {
            assert_eq!(source.get(field), current.get(field));
        }
        let no_key = {
            let mut c = config.clone();
            c.encryption_key_previous = None;
            c.encryption_key = Some("33".repeat(32));
            EncryptionKeys::from_config(&c)
        };
        assert!(migrate(entry, &db, &no_key, &current).await.is_err());
        for (k, v) in [
            (
                "secret_context_versions",
                Bson::Document(doc! {REVOCATION:7,CONNECTION:3}),
            ),
            (REVOCATION, Bson::Null),
            (REVOCATION, bytes(vec![3, b'N', b'Y', b'X'])),
            (CONNECTION, Bson::String("invalid".into())),
        ] {
            let mut bad = current.clone();
            bad.insert(k, v);
            assert!(verify(entry, &db, &keys, &bad).await.is_err());
            assert!(migrate(entry, &db, &keys, &bad).await.is_err());
        }
        let mut bad = current.clone();
        let mut oversized = current.get_binary_generic(REVOCATION).unwrap().clone();
        oversized[5..7].copy_from_slice(&1025u16.to_be_bytes());
        bad.insert(REVOCATION, bytes(oversized));
        assert!(migrate(entry, &db, &keys, &bad).await.is_err());
        rows.delete_one(lookup.clone()).await.unwrap();
    }
}

#[tokio::test]
async fn compatible_partial_writers_clear_only_the_replaced_marker() {
    use crate::services::{
        developer_webhook_service as connection, oauth_client_service as clients,
    };
    let db = crate::test_utils::connect_transaction_test_database("webhook_partial").await;
    let current = crate::test_utils::test_encryption_keys();
    let mut compatible = crate::test_utils::test_encryption_keys();
    compatible.set_contextual_record_writes(false);
    let scope = crate::models::oauth_client::ScopeProvenance::Explicit;
    let (client, _) = clients::create_client(
        &db,
        "fixture",
        &[],
        "public",
        "org",
        "",
        "openid",
        scope,
        false,
        None,
        Some((&current, "revocation-1")),
        &[],
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let url = format!("http://{address}/hook");
    let (both, _, _) = crate::services::webhook_delivery_service::with_test_webhook_target(
        address,
        connection::configure(&db, &current, &client.id, "org", &url),
    )
    .await
    .unwrap();
    assert_eq!(both.secret_context_versions.len(), 2);
    let updated = clients::update_client_for_creator(
        &db,
        &client.id,
        "org",
        None,
        None,
        None,
        None,
        None,
        None,
        Some((&compatible, "revocation-2")),
        None,
    )
    .await
    .unwrap();
    assert!(!updated.secret_context_versions.contains_key(REVOCATION));
    assert_eq!(updated.secret_context_versions.get(CONNECTION), Some(&3));
    assert_eq!(
        updated.connection_webhook_secret_encrypted,
        both.connection_webhook_secret_encrypted
    );
    assert_eq!(
        &**oauth_client_webhook_record::open(
            &compatible,
            &updated,
            REVOCATION,
            updated
                .revocation_webhook_secret_encrypted
                .as_deref()
                .unwrap()
        )
        .await
        .unwrap(),
        b"revocation-2"
    );
    let (rotated, raw, _) = connection::rotate_secret(&db, &compatible, &client.id, "org")
        .await
        .unwrap();
    assert!(rotated.secret_context_versions.is_empty());
    assert_eq!(
        &**oauth_client_webhook_record::open(
            &compatible,
            &rotated,
            CONNECTION,
            rotated
                .connection_webhook_secret_encrypted
                .as_deref()
                .unwrap()
        )
        .await
        .unwrap(),
        raw.as_bytes()
    );
    let (active, _, _) = connection::rotate_secret(&db, &current, &client.id, "org")
        .await
        .unwrap();
    assert_eq!(active.secret_context_versions.get(CONNECTION), Some(&3));
    let disabled = connection::disable(&db, &client.id, "org").await.unwrap();
    assert!(disabled.secret_context_versions.is_empty());
    assert_eq!(
        disabled.revocation_webhook_secret_encrypted,
        rotated.revocation_webhook_secret_encrypted
    );
}

#[tokio::test]
async fn webhook_generation_resumes_with_separate_field_outcomes() {
    crate::test_utils::init_test_audit_chain_hmac_key();
    let db = crate::test_utils::connect_transaction_test_database("webhook_generation").await;
    let keys = crate::test_utils::test_encryption_keys();
    let index = REGISTRY
        .iter()
        .position(|e| e.collection == "oauth_clients")
        .unwrap();
    let entry = &REGISTRY[index];
    let rows = db.collection::<Document>(entry.collection);
    for i in 0..=BATCH {
        rows.insert_one(doc! {"_id":format!("row-{i:02}"),"created_by":"owner",
        REVOCATION:bytes(hex::decode(LEGACY_REVOCATION).unwrap())})
            .await
            .unwrap();
    }
    let bad = doc! {"_id":"zz-last","created_by":"owner", CONNECTION:bytes(hex::decode(INVALID_UTF8_CONNECTION).unwrap())};
    rows.insert_one(&bad).await.unwrap();
    let generation = start(&db, &keys, Mode::Migrate, None).await.unwrap();
    let runs = db.collection::<Document>(RUNS);
    runs.update_one(doc! {"_id":RUN_ID}, doc! {"$set":{"entry":index as i32}})
        .await
        .unwrap();
    // A successful CAS before a progress checkpoint: the resumed walker must
    // authenticate this winner without counting or performing a second rewrite.
    let before = rows.find_one(doc! {"_id":"row-00"}).await.unwrap().unwrap();
    assert!(migrate(entry, &db, &keys, &before).await.unwrap());
    advance(&db, &keys, &generation).await.unwrap();
    let saved = runs.find_one(doc! {"_id":RUN_ID}).await.unwrap().unwrap();
    assert_eq!(saved.get_str("cursor"), Ok("row-15"));
    advance(&db, &keys, &generation).await.unwrap();
    let report = status(&db, &keys).await.unwrap().unwrap();
    assert_eq!(report.unresolved, 1);
    assert_eq!(report.rewritten, BATCH);
    let outcome = |field| {
        report
            .field_outcomes
            .iter()
            .find(|o| o.collection == entry.collection && o.field == field)
            .unwrap()
    };
    assert_eq!(outcome(REVOCATION).authenticated_current, BATCH + 1);
    assert_eq!(outcome(CONNECTION).no_material_observed, BATCH + 1);
    assert_eq!(outcome(CONNECTION).verification_blocked, 1);
    assert_eq!(
        rows.find_one(doc! {"_id":"zz-last"}).await.unwrap(),
        Some(bad)
    );
}
