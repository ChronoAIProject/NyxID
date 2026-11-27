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

// Unmarked v2 fixtures from the server tests at 846b09d4d.
// The prior writer stored the field name itself in each envelope.
fn legacy_field(field: &str) -> Vec<u8> {
    let ciphertext = match field {
        "registration_pin_encrypted" => concat!(
            "0202003cc6f0b2eb61cb3c63c86bd79d9e23adebe9f6c06c5b1e7f7436284d8cb0fde761298a7bcb",
            "1454a5fbeaf19a12c41798622f82c609b85652c6f5aad76a01b6e95e2ffd12224c0955dde6fe7733",
            "903806b72de1cc5fd8d4036bcf37c7802b4c01cbfea3c21286295d80079ad66c25a251a8656e",
        ),
        "webhook_secret_encrypted" => concat!(
            "0202003c000d512956feb4cda699224d4470411b19d5bde81a7d77313cc8f99912904901c8cc2b4e",
            "64a135459de68d6a5954fde1989123dae64810ed3fe36d03acbe870ef78b4aa4f001b6fc4ada7f68",
            "b60301d97b690f9ced076b447807eac6bb6c76377c3fec7dc9e59027bf3c646c6f134af9",
        ),
        "bot_token_encrypted" => concat!(
            "0202003cb6c91c2354f33949123472cbd93eacb3cbbe9b2dad0faf8e7451497bda8885efbf428f99",
            "ecbed9e0fd16ebc87f2171e6d96f1654bc7df5defdceb5eccb7a58ef76310690aa6ca3444387473d",
            "c9be0adbbab8153efe698f1faaf56594a45d279a915cf6ffbfc6566b7eea48",
        ),
        "app_secret_encrypted" => concat!(
            "0202003ca4f85e4ccf88b53e385b68ef6896ecf0370d66d55d13f77be0fef7847e4267c699117796",
            "40dee01677053c877585c95c08db96fd823e68c0dff1514740ba822c1c87c25cba9dfb814c29df49",
            "31199932aa1210eee3cbc4b6b4bf533d1322b7010c87aa7d4a2afd13341e4ecc",
        ),
        "lark_verification_token_encrypted" => concat!(
            "0202003c124d9cda71c1e4795a4af9b433818504d1dfec2be17f43f7e34745b093b509ef5769cd36",
            "1798311e50091748ed02c31049f8df56992bef4d37020321ce5b1a1ad217f3e7557bfdb8bd483047",
            "886edff5c5279a8a541794fd6cbf57e9ab097a0da8996474b3b18361173f18179449d721fde04422",
            "3fd404515f",
        ),
        "lark_encrypt_key_encrypted" => concat!(
            "0202003c7dffbcce5713f97aedb31ef62379f22c841266276c0aeea36fe76824b061ca9ca060888a",
            "f40642f67a756ab0108e3339c749b76e1753bd88d1cd41b04bbeed4fbdc48f3cb9b9d493b29fdbe5",
            "65d03a0cfb25f9190cf577b50f179afaf7fd5c42d5496f88315c1858d662e722cdb12616bc6e",
        ),
        _ => panic!("missing legacy channel bot fixture for {field}"),
    };
    hex::decode(ciphertext).unwrap()
}

fn legacy(id: &str) -> Document {
    let now = bson::DateTime::now();
    let mut row = doc! {"_id":id,"user_id":"owner","platform":"lark","platform_bot_id":"app","platform_bot_username":"fixture",
    "is_active":false,"status":"inactive","label":"test","created_at":now,"updated_at":now,"webhook_registered":false,
    "webhook_secret_hash":hex::encode(Sha256::digest(b"webhook_secret_encrypted")),"unmodeled_field":42};
    for &field in FIELDS {
        let bytes = legacy_field(field);
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
    let row = legacy("bot");
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
    let source = legacy("first");
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
