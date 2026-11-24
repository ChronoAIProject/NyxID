use super::*;
use crate::services::oauth_client_webhook_record::{CONNECTION, REVOCATION};

use super::swaps::assert_registry_swaps;

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
    REVOCATION: bson::to_bson(&old.encrypt(b"revocation fixture").await.unwrap()).unwrap(),
    CONNECTION: bytes(old.encrypt(b"connection fixture").await.unwrap())};
    rows.insert_one(&source).await.unwrap();
    let lookup = doc! {"_id":"row"};
    // The snapshot is from before each competing operation. Even a change to
    // representation alone is an exact-source conflict.
    for update in [
        doc! {"$set":{"created_by":"org"}},
        doc! {"$set":{CONNECTION:bytes(old.encrypt(b"rotated").await.unwrap()),"connection_webhook_key_id":"key-2"}},
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
        &**compatible
            .decrypt(
                updated
                    .revocation_webhook_secret_encrypted
                    .as_ref()
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
        &**compatible
            .decrypt(
                rotated
                    .connection_webhook_secret_encrypted
                    .as_ref()
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
        REVOCATION:bytes(keys.encrypt(b"legacy").await.unwrap())})
            .await
            .unwrap();
    }
    let bad = doc! {"_id":"zz-last","created_by":"owner", CONNECTION:bytes(keys.encrypt(&[0xff]).await.unwrap())};
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
