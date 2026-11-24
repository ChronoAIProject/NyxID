use super::swaps::assert_registry_swaps;
use super::*;
use crate::services::channel_bot_record::FIELDS;
use sha2::{Digest, Sha256};

fn binary(bytes: Vec<u8>) -> Bson {
    Bson::Binary(bson::Binary {
        subtype: bson::spec::BinarySubtype::Generic,
        bytes,
    })
}

async fn legacy(keys: &EncryptionKeys, id: &str) -> Document {
    let now = bson::DateTime::now();
    let mut row = doc! {"_id":id,"user_id":"owner","platform":"lark","platform_bot_id":"app","platform_bot_username":"fixture",
    "is_active":false,"status":"inactive","label":"test","created_at":now,"updated_at":now,"webhook_registered":false,
    "webhook_secret_hash":hex::encode(Sha256::digest(b"webhook_secret_encrypted")),"unmodeled_field":42};
    for &field in FIELDS {
        let bytes = keys.encrypt(field.as_bytes()).await.unwrap();
        row.insert(
            field,
            if field == "bot_token_encrypted" {
                bson::to_bson(&bytes).unwrap()
            } else {
                binary(bytes)
            },
        );
    }
    row
}

#[tokio::test]
async fn inactive_channel_bot_rewrap_preserves_pins_and_rejects_stale_sources_and_swaps() {
    let db = crate::test_utils::connect_transaction_test_database("bot_rewrap_contract").await;
    let entry = REGISTRY
        .iter()
        .find(|e| e.collection == "channel_bots")
        .unwrap();
    let rows = db.collection::<Document>(entry.collection);
    let mut config = crate::test_utils::test_app_config();
    let old = EncryptionKeys::from_config(&config);
    config.encryption_key_previous = config.encryption_key.replace("22".repeat(32));
    let new = EncryptionKeys::from_config(&config);
    let row = legacy(&old, "bot").await;
    let id = doc! {"_id":"bot"};
    rows.insert_one(&row).await.unwrap();
    for mutation in [
        doc! {"user_id":"next"},
        doc! {"ownership_version":1},
        doc! {"registration_pin_encrypted":Bson::Null},
        doc! {"is_active":true},
        doc! {"bot_token_encrypted":binary(crate::services::record_field_service::stored_bytes(row.get("bot_token_encrypted").unwrap()).unwrap())},
    ] {
        rows.update_one(id.clone(), doc! {"$set":mutation})
            .await
            .unwrap();
        let winner = rows.find_one(id.clone()).await.unwrap().unwrap();
        assert!(!migrate(entry, &db, &old, &row).await.unwrap());
        assert!(rows.find_one(id.clone()).await.unwrap().unwrap() == winner);
        rows.replace_one(id.clone(), &row).await.unwrap();
    }
    assert!(migrate(entry, &db, &old, &row).await.unwrap());
    let staged = rows.find_one(id.clone()).await.unwrap().unwrap();
    assert!(verify(entry, &db, &new, &staged).await.is_err());
    assert!(migrate(entry, &db, &new, &staged).await.unwrap());
    let current = rows.find_one(id.clone()).await.unwrap().unwrap();
    assert_eq!(current.get_i32("unmodeled_field"), Ok(42));
    assert_eq!(current.get("updated_at"), row.get("updated_at"));
    for name in ["pins", "generations"] {
        assert!(
            current
                .get_document("credential_continuity")
                .unwrap()
                .get(name)
                == staged
                    .get_document("credential_continuity")
                    .unwrap()
                    .get(name)
        );
    }
    assert_registry_swaps(entry, &db, &new, &current, "user_id").await;
    assert!(!migrate(entry, &db, &new, &current).await.unwrap());
    assert!(migrate(entry, &db, &old, &current).await.is_err());
    for (name, value) in [
        ("webhook_secret_hash", "bad".into()),
        (
            "secret_context_versions",
            doc! {"registration_pin_encrypted":4}.into(),
        ),
        ("registration_pin_encrypted", Bson::Null),
        ("app_secret_encrypted", binary(vec![3, 1])),
        ("lark_verification_token_encrypted", "wrong type".into()),
    ] {
        let mut damaged = current.clone();
        damaged.insert(name, value);
        assert!(verify(entry, &db, &new, &damaged).await.is_err(), "{name}");
        assert!(migrate(entry, &db, &new, &damaged).await.is_err(), "{name}");
    }
    let mut wrong_hash = row.clone();
    wrong_hash.insert("webhook_secret_hash", "mismatch");
    assert!(migrate(entry, &db, &new, &wrong_hash).await.is_err());
    let mut empty = row.clone();
    for &f in FIELDS {
        empty.remove(f);
    }
    empty.insert("app_secret_encrypted", Bson::Null);
    empty.insert("bot_token_encrypted", binary(vec![]));
    assert!(verify(entry, &db, &new, &empty).await.is_err());
    for kind in ["connection", "telegram_manager"] {
        empty.insert("credential_source", kind);
        verify(entry, &db, &new, &empty).await.unwrap();
        assert!(!migrate(entry, &db, &new, &empty).await.unwrap());
    }
    rows.delete_one(id.clone()).await.unwrap();
    assert!(!migrate(entry, &db, &new, &row).await.unwrap());
    assert!(rows.find_one(id).await.unwrap().is_none());
}

#[tokio::test]
async fn channel_bot_generation_resumes_after_rewrap_before_checkpoint() {
    let db = crate::test_utils::connect_transaction_test_database("bot_generation").await;
    let keys = crate::test_utils::test_encryption_keys();
    let index = REGISTRY
        .iter()
        .position(|e| e.collection == "channel_bots")
        .unwrap();
    let entry = &REGISTRY[index];
    let rows = db.collection::<Document>(entry.collection);
    let source = legacy(&keys, "first").await;
    let mut optional = source.clone();
    optional.insert("_id", "optional");
    optional.remove("webhook_secret_encrypted");
    optional.insert("registration_pin_encrypted", Bson::Null);
    let mut damaged = source.clone();
    damaged.insert("_id", "damaged");
    damaged.insert("app_secret_encrypted", binary(vec![3, 1]));
    rows.insert_many([&source, &optional, &damaged])
        .await
        .unwrap();
    let generation = start(&db, &keys, Mode::Migrate, None).await.unwrap();
    db.collection::<Document>(RUNS)
        .update_one(doc! {"_id":RUN_ID}, doc! {"$set":{"entry":index as i32}})
        .await
        .unwrap();
    assert!(migrate(entry, &db, &keys, &source).await.unwrap());
    assert!(advance(&db, &keys, "stale-generation").await.is_err());
    advance(&db, &keys, &generation).await.unwrap();
    let report = status(&db, &keys).await.unwrap().unwrap();
    assert_eq!(
        (report.scanned, report.rewritten, report.unresolved),
        (3, 1, 1)
    );
    for &name in FIELDS {
        let outcome = report
            .field_outcomes
            .iter()
            .find(|f| f.collection == entry.collection && f.field == name)
            .unwrap();
        let absent =
            i64::from(["webhook_secret_encrypted", "registration_pin_encrypted"].contains(&name));
        assert_eq!(
            (
                outcome.authenticated_current,
                outcome.no_material_observed,
                outcome.verification_blocked
            ),
            (2 - absent, absent, 1)
        );
    }
    assert!(
        rows.find_one(doc! {"_id":"damaged"})
            .await
            .unwrap()
            .unwrap()
            == damaged
    );
}
