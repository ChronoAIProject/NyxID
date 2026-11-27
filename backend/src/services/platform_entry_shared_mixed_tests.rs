//! The same fixture is compiled against each binary's own readers, writers and walker.
use super::*;
use crate::services::{
    encrypted_record_migration as migration, platform_credential_service as platform,
    platform_entry_credentials::EntryCredentialSnapshot,
};
use bson::{Document, doc};

#[tokio::test]
async fn shared_signing_mixed_records() {
    let Ok(file) = std::env::var("NYXID_RECORD_FIXTURE_FILE") else {
        return;
    };
    let step = std::env::var("NYXID_RECORD_FIXTURE_STEP").unwrap();
    let writes = std::env::var("CONTEXTUAL_RECORD_WRITES").as_deref() == Ok("true");
    let mut keys = crate::test_utils::test_encryption_keys();
    keys.set_contextual_record_writes(writes);
    keys.set_allow_legacy_record_reads(
        std::env::var("ALLOW_LEGACY_RECORD_READS").as_deref() != Ok("false"),
    );
    let adapter = crate::services::channel_adapters::x::XAdapter::default();
    let descriptor = adapter.platform_credentials().unwrap();
    crate::test_utils::init_test_audit_chain_hmac_key();
    let collections = [
        "channel_bots",
        "users",
        "user_api_keys",
        "provider_configs",
        "platform_credentials",
    ];
    let (db, mut data) = if step == "write" {
        let (state, bot, _, _) = stored_x_fixture().await;
        let values = [
            ("client_id", "client"),
            ("client_secret", "secret"),
            ("consumer_secret", "consumer"),
            ("app_bearer_token", "app"),
        ]
        .map(|(k, v)| (k.into(), Some(Zeroizing::new(v.into()))))
        .into();
        platform::update(&state.db, &keys, &descriptor, "fixture", &values, false)
            .await
            .unwrap();
        (state.db, doc! {"bot":bot.id,"client_secret":"secret"})
    } else {
        let data: Document = bson::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        let db = crate::test_utils::connect_transaction_test_database("mixed_signing_reader").await;
        for coll in collections {
            db.collection::<Document>(coll)
                .insert_one(data.get_document(coll).unwrap())
                .await
                .unwrap();
        }
        (db, data)
    };
    let read =
        crate::services::channel_bot_service::get_bot(&db, data.get_str("bot").unwrap()).await;
    if step == "reject-shared" {
        assert!(matches!(
            read,
            Err(crate::errors::AppError::DatabaseError(_))
        ));
        return;
    }
    let mut bot = read.unwrap();
    let before = EntryCredentialSnapshot::load(&db, &adapter, &bot)
        .await
        .unwrap();
    if step != "write" {
        assert_eq!(before.fingerprint().unwrap(), data.get_str("pin").unwrap());
    }
    match step.as_str() {
        "write" => {
            data.insert("pin", before.fingerprint().unwrap());
            for collection in ["provider_configs", "platform_credentials"] {
                let row = db
                    .collection::<Document>(collection)
                    .find_one(doc! {})
                    .await
                    .unwrap()
                    .unwrap();
                assert!(!row.contains_key("channel_signing_preparation"));
            }
        }
        "replace" | "clear" => {
            let value = (step == "replace").then(|| Zeroizing::new("same replacement".into()));
            platform::update(
                &db,
                &keys,
                &descriptor,
                "fixture",
                &[("client_secret".into(), value)].into(),
                false,
            )
            .await
            .unwrap();
            let successor = EntryCredentialSnapshot::load(&db, &adapter, &bot)
                .await
                .unwrap()
                .fingerprint()
                .unwrap();
            if step == "replace" || data.contains_key("client_secret") {
                assert_ne!(successor, data.get_str("pin").unwrap());
            }
            data.insert("pin", successor);
            if step == "replace" {
                data.insert("client_secret", "same replacement");
            } else {
                data.remove("client_secret");
            }
        }
        "rewrap-pinned" => {
            let generation = migration::start(&db, &keys, migration::Mode::Migrate, None)
                .await
                .unwrap();
            for _ in 0..100 {
                let report = migration::status(&db, &keys).await.unwrap().unwrap();
                if report.phase == "observed" {
                    break;
                }
                migration::advance(&db, &keys, &generation).await.unwrap();
            }
            let report = migration::status(&db, &keys).await.unwrap().unwrap();
            assert_eq!(report.phase, "observed");
            for collection in ["provider_configs", "platform_credentials"] {
                let row = db
                    .collection::<Document>(collection)
                    .find_one(doc! {})
                    .await
                    .unwrap()
                    .unwrap();
                assert!(
                    row.get_document("channel_signing_preparation")
                        .unwrap()
                        .get_bool("complete")
                        .unwrap()
                );
                assert!(
                    !row.get_document("secret_context_versions")
                        .unwrap()
                        .is_empty()
                );
            }
            bot = crate::services::channel_bot_service::get_bot(&db, &bot.id)
                .await
                .unwrap();
            assert_eq!(
                EntryCredentialSnapshot::load(&db, &adapter, &bot)
                    .await
                    .unwrap()
                    .fingerprint()
                    .unwrap(),
                data.get_str("pin").unwrap()
            );
        }
        "read" | "read-pinned-strict" => {}
        _ => panic!("unknown fixture step"),
    }
    let snapshot = EntryCredentialSnapshot::load(&db, &adapter, &bot)
        .await
        .unwrap();
    let secrets = snapshot.secrets(&keys, &adapter).await.unwrap();
    assert_eq!(
        secrets.get("client_secret"),
        data.get_str("client_secret").ok()
    );
    assert_eq!(secrets.get("consumer_secret"), Some("consumer"));
    for coll in collections {
        data.insert(
            coll,
            db.collection::<Document>(coll)
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap(),
        );
    }
    std::fs::write(file, bson::to_vec(&data).unwrap()).unwrap();
}
