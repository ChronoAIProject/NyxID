//! Cross-revision fixture. Only test credentials and their digest cross processes.
use super::*;
use bson::Document;
use sha2::{Digest, Sha256};

#[tokio::test]
async fn whatsapp_mixed_records() {
    let Ok(step) = std::env::var("NYXID_RECORD_FIXTURE_STEP") else {
        return;
    };
    let file = std::env::var("NYXID_RECORD_FIXTURE_FILE").unwrap();
    let db = crate::test_utils::connect_transaction_test_database("whatsapp_mixed").await;
    let mut settings = crate::test_utils::test_app_config();
    settings.contextual_record_writes =
        std::env::var("CONTEXTUAL_RECORD_WRITES").as_deref() == Ok("true");
    settings.allow_legacy_record_reads =
        std::env::var("ALLOW_LEGACY_RECORD_READS").as_deref() != Ok("false");
    let active = settings.contextual_record_writes;
    let keys = EncryptionKeys::from_config(&settings);
    let rows = db.collection::<Document>(COLLECTION_NAME);
    let mut fixture: Document = if step == "write" {
        doc! {"parent":bson::to_document(&tests::bot()).unwrap()}
    } else {
        bson::from_slice(&std::fs::read(&file).unwrap()).unwrap()
    };
    let mut bot: ChannelBot =
        bson::from_document(fixture.get_document("parent").unwrap().clone()).unwrap();
    if let Ok(stored) = fixture.get_document("row") {
        let mut row = stored.clone();
        match step.as_str() {
            "swap-owner" => {
                bot.user_id = uuid::Uuid::new_v4().to_string();
                row.insert("owner_id", &bot.user_id);
            }
            "swap-id" => {
                bot.id = uuid::Uuid::new_v4().to_string();
                row.insert("_id", &bot.id);
            }
            "swap-generation" => {
                bot.ownership_version += 1;
                row.insert("ownership_version", bot.ownership_version);
            }
            _ => {}
        }
        rows.insert_one(row).await.unwrap();
    }
    db.collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .insert_one(&bot)
        .await
        .unwrap();
    let id = doc! {"_id":&bot.id};
    if step == "migrate" {
        use crate::services::encrypted_record_migration::{Mode, advance, start, status};
        let generation = start(&db, &keys, Mode::Migrate, None).await.unwrap();
        loop {
            let report = status(&db, &keys).await.unwrap().unwrap();
            if report.collection == Some(COLLECTION_NAME) {
                advance(&db, &keys, &generation).await.unwrap();
                let report = status(&db, &keys).await.unwrap().unwrap();
                assert!(
                    report
                        .field_outcomes
                        .iter()
                        .any(|outcome| outcome.collection == COLLECTION_NAME
                            && outcome.authenticated_current == 1)
                );
                break;
            }
            assert_eq!(report.phase, "running");
            advance(&db, &keys, &generation).await.unwrap();
        }
        fixture.insert("row", rows.find_one(id).await.unwrap().unwrap());
        std::fs::write(file, bson::to_vec(&fixture).unwrap()).unwrap();
    } else if matches!(step.as_str(), "write" | "replace" | "clear") {
        let revision = load(&db, &bot).await.unwrap().map_or(0, |c| c.revision);
        let config = configure(&db, &bot, "111", "222", true, revision)
            .await
            .unwrap();
        fixture.remove("digest");
        if step != "clear" {
            let token = native_verify_token(&db, &keys, &config).await.unwrap();
            fixture.insert("digest", hex::encode(Sha256::digest(token.as_bytes())));
        }
        let row = rows.find_one(id).await.unwrap().unwrap();
        assert_eq!(
            row.get_document("secret_context_versions")
                .ok()
                .and_then(|m| m.get_i32("native_verify_token_encrypted").ok()),
            (active && step != "clear").then_some(3)
        );
        if step == "clear" {
            assert_eq!(
                row.get("native_verify_token_encrypted"),
                Some(&bson::Bson::Null)
            );
        } else {
            assert!(
                row.get_binary_generic("native_verify_token_encrypted")
                    .is_ok()
            );
        }
        fixture.insert("row", row);
        std::fs::write(file, bson::to_vec(&fixture).unwrap()).unwrap();
    } else {
        let config = load(&db, &bot).await.unwrap().unwrap();
        if fixture.contains_key("digest") {
            let result = native_verify_token(&db, &keys, &config).await;
            if step == "read" {
                assert_eq!(
                    hex::encode(Sha256::digest(result.unwrap().as_bytes())),
                    fixture.get_str("digest").unwrap()
                );
            } else {
                assert!(step == "reject" || step.starts_with("swap-"));
                assert!(result.is_err());
            }
        } else {
            assert_eq!(step, "read");
            assert!(config.native_verify_token_encrypted.is_none());
        }
    }
    db.drop().await.unwrap();
}
