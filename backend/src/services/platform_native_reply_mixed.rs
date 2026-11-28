use super::*;
use crate::crypto::record_field::historical::HistoricalAad;
use crate::{
    crypto::aes::EncryptionKeys,
    models::platform_entry_identity::NATIVE_RECEIPTS,
    test_utils::{
        connect_transaction_test_database, run_native_journey_test, test_app_state,
        test_encryption_keys,
    },
};
use bson::Document;
use futures::TryStreamExt;
use std::{collections::BTreeMap, path::Path, sync::Arc};

fn fixture_keys() -> EncryptionKeys {
    let mut keys = test_encryption_keys();
    keys.set_contextual_record_writes(
        std::env::var("CONTEXTUAL_RECORD_WRITES").as_deref() == Ok("true"),
    );
    keys.set_allow_legacy_record_reads(
        std::env::var("ALLOW_LEGACY_RECORD_READS").as_deref() != Ok("false"),
    );
    keys
}

async fn save(state: &AppState, path: &Path) {
    use crate::models::{channel_bot, platform_bot_config, platform_bot_runtime_controls, user};
    let mut rows = BTreeMap::new();
    for name in [
        user::COLLECTION_NAME,
        channel_bot::COLLECTION_NAME,
        platform_bot_config::COLLECTION_NAME,
        platform_bot_runtime_controls::COLLECTION_NAME,
        NATIVE_RECEIPTS,
        HANDLES,
    ] {
        let documents: Vec<Document> = state
            .db
            .collection(name)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        rows.insert(name, documents);
    }
    std::fs::write(path, bson::to_vec(&rows).unwrap()).unwrap();
}

async fn restore(path: &Path, keys: EncryptionKeys) -> (AppState, NativeEntryReceipt) {
    let db = connect_transaction_test_database("mixed_reply_handle").await;
    let mut state = test_app_state(db.clone());
    state.encryption_keys = Arc::new(keys);
    crate::services::platform_entry_migration::ensure(&db)
        .await
        .unwrap();
    let rows: BTreeMap<String, Vec<Document>> =
        bson::from_slice(&std::fs::read(path).unwrap()).unwrap();
    for (name, docs) in rows {
        db.collection::<Document>(&name)
            .insert_many(docs)
            .await
            .unwrap();
    }
    let receipt = db
        .collection::<NativeEntryReceipt>(NATIVE_RECEIPTS)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    (state, receipt)
}

#[test]
fn mixed_signed_discord_handle() {
    run_native_journey_test("mixed_signed_discord_handle", || async {
        let step =
            std::env::var("NYXID_RECORD_FIXTURE_STEP").unwrap_or_else(|_| "roundtrip".into());
        let file = std::env::var_os("NYXID_RECORD_FIXTURE_FILE");
        let server = MockServer::start().await;
        let (state, receipt) = match step.as_str() {
            "write" | "roundtrip" => {
                let (state, _, receipt) =
                    discord_fixture_with_keys(&server.uri(), fixture_keys()).await;
                if let Some(file) = &file {
                    save(&state, file.as_ref()).await;
                    assert!(server.received_requests().await.unwrap().is_empty());
                    return;
                }
                (state, receipt)
            }
            "read" | "reject" => restore(file.as_ref().unwrap().as_ref(), fixture_keys()).await,
            _ => panic!("unknown fixture step"),
        };
        let adapter =
            crate::services::channel_adapters::discord::DiscordAdapter::media_test_adapter(
                &server.uri(),
            );
        let handle = state.db.collection::<Document>(HANDLES);
        let before = handle
            .find_one(doc! {"receipt_id":&receipt.id})
            .await
            .unwrap()
            .unwrap();
        Mock::given(method("POST"))
            .and(path("/webhooks/100/synthetic-interaction-token"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"id":"fixture-send"})),
            )
            .mount(&server)
            .await;
        let authority = claim(&state, &receipt, &adapter).await.unwrap();
        let outcome = send_claimed_for_test(
            &state,
            authority,
            Zeroizing::new("fixture reply".into()),
            &adapter,
            &server.uri(),
        )
        .await
        .unwrap();
        let requests = server.received_requests().await.unwrap();
        if step == "reject" {
            assert!(matches!(outcome, NativeSendOutcome::Refused));
            assert!(requests.is_empty());
        } else {
            assert!(
                matches!(outcome, NativeSendOutcome::Sent{platform_message_id} if platform_message_id == "fixture-send")
            );
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].url.query(), Some("wait=true"));
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&requests[0].body).unwrap()["content"],
                "fixture reply"
            );
        }
        assert!(
            handle
                .find_one(doc! {"receipt_id":&receipt.id})
                .await
                .unwrap()
                .unwrap()
                == before
        );
    });
}

// Candidate storage tests
async fn one_handle(state: &AppState, receipt: &NativeEntryReceipt) -> Document {
    state
        .db
        .collection::<Document>(HANDLES)
        .find_one(doc! {"receipt_id": &receipt.id})
        .await
        .unwrap()
        .unwrap()
}

#[test]
fn rewrap_captured_reply_handle() {
    run_native_journey_test("rewrap_captured_reply_handle", || async {
        let server = MockServer::start().await;
        let file = std::env::var_os("NYXID_RECORD_FIXTURE_FILE");
        let (state, receipt) = if let Some(file) = &file {
            restore(file.as_ref(), test_encryption_keys()).await
        } else {
            let mut legacy = test_encryption_keys();
            legacy.set_contextual_record_writes(false);
            let (state, _, receipt) = discord_fixture_with_keys(&server.uri(), legacy).await;
            (state, receipt)
        };
        let adapter =
            crate::services::channel_adapters::discord::DiscordAdapter::media_test_adapter(
                &server.uri(),
            );
        let claimed = claim(&state, &receipt, &adapter).await.unwrap();
        let before = one_handle(&state, &receipt).await;
        assert!(!before.contains_key("ciphertext_context_version"));
        let keys = test_encryption_keys();
        let aad = binding_digest(&receipt).unwrap();
        let original = HistoricalAad::native_reply(&aad)
            .unwrap()
            .open(&keys, before.get_binary_generic("ciphertext").unwrap())
            .await
            .unwrap();
        assert!(record::migrate(&state.db, &keys, &before).await.unwrap());
        assert!(!record::migrate(&state.db, &keys, &before).await.unwrap());
        let after = one_handle(&state, &receipt).await;
        record::verify(&state.db, &keys, &after).await.unwrap();
        assert!(!record::migrate(&state.db, &keys, &after).await.unwrap());
        let mut metadata = after.clone();
        assert_eq!(
            metadata.remove("ciphertext_context_version"),
            Some(bson::Bson::Int32(3))
        );
        metadata.insert("ciphertext", before.get("ciphertext").unwrap().clone());
        assert!(metadata == before);

        let created = after
            .get_datetime("created_at")
            .unwrap()
            .timestamp_millis()
            .to_string();
        let expires = after
            .get_datetime("expires_at")
            .unwrap()
            .timestamp_millis()
            .to_string();
        let parts = [
            "ciphertext",
            &receipt.profile_id,
            &receipt.id,
            &aad,
            &created,
            &expires,
        ];
        let context = crate::crypto::record_field::RecordContext::new(
            HANDLES,
            after.get_str("_id").unwrap(),
            "platform",
            &parts,
        )
        .unwrap();
        let plain = keys
            .decrypt_record(after.get_binary_generic("ciphertext").unwrap(), &context)
            .await
            .unwrap();
        assert!(plain.as_slice() == original.as_slice());
        assert!(server.received_requests().await.unwrap().is_empty());
        if let Some(file) = &file {
            save(&state, file.as_ref()).await;
        }
        // An existing dispatch owns the exact pair it captured, even after a
        // storage rewrite and subsequent removal of both retained documents.
        state
            .db
            .collection::<Document>(HANDLES)
            .delete_one(doc! {"_id":after.get("_id").unwrap()})
            .await
            .unwrap();
        state
            .db
            .collection::<Document>(NATIVE_RECEIPTS)
            .delete_one(doc! {"_id":&receipt.id})
            .await
            .unwrap();
        Mock::given(method("POST"))
            .and(path("/webhooks/100/synthetic-interaction-token"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"id":"captured"})),
            )
            .mount(&server)
            .await;
        let result = send_claimed_for_test(
            &state,
            claimed,
            Zeroizing::new("already claimed".into()),
            &adapter,
            &server.uri(),
        )
        .await
        .unwrap();
        assert!(
            matches!(result, NativeSendOutcome::Sent{platform_message_id} if platform_message_id=="captured")
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        assert!(claim(&state, &receipt, &adapter).await.is_err());
    });
}

#[test]
fn both_ttl_orders_and_missing_receipt_finding() {
    run_native_journey_test("both_ttl_orders_and_missing_receipt_finding", || async {
        use crate::services::encrypted_record_migration as migration;
        crate::test_utils::init_test_audit_chain_hmac_key();
        let server = MockServer::start().await;
        let (state, _, receipt) = discord_fixture(&server.uri()).await;
        let before = one_handle(&state, &receipt).await;
        let db = &state.db;
        let keys = &state.encryption_keys;
        let adapter =
            crate::services::channel_adapters::discord::DiscordAdapter::media_test_adapter(
                &server.uri(),
            );
        let handles = db.collection::<Document>(HANDLES);
        let select = doc! {"_id":before.get("_id").unwrap()};
        handles.delete_one(select.clone()).await.unwrap();
        assert!(claim(&state, &receipt, &adapter).await.is_err());
        let mut config = crate::test_utils::test_app_config();
        config.encryption_key_previous = config.encryption_key.replace("22".repeat(32));
        let rotated = EncryptionKeys::from_config(&config);
        assert!(!record::migrate(db, &rotated, &before).await.unwrap());
        assert!(handles.find_one(select.clone()).await.unwrap().is_none());
        handles.insert_one(&before).await.unwrap();
        db.collection::<Document>(NATIVE_RECEIPTS)
            .delete_one(doc! {"_id":&receipt.id})
            .await
            .unwrap();
        assert!(claim(&state, &receipt, &adapter).await.is_err());
        assert!(record::verify(db, keys, &before).await.is_err());
        let generation = migration::start(db, keys, migration::Mode::Migrate, None)
            .await
            .unwrap();
        let finding = loop {
            migration::advance(db, keys, &generation).await.unwrap();
            if let Some(f) = migration::list_findings(db, None)
                .await
                .unwrap()
                .findings
                .into_iter()
                .find(|f| f.collection == HANDLES)
            {
                break f;
            }
            assert_eq!(
                migration::status(db, keys).await.unwrap().unwrap().phase,
                "running"
            );
        };
        assert_eq!(finding.category, record::RECEIPT_MISSING);
        let lookup = migration::lookup_finding(db, keys, &finding.id, None, true)
            .await
            .unwrap();
        assert_eq!(lookup.disposition, "storage_or_lifecycle_repair_required");
        assert!(lookup.record_uuid.is_none() && lookup.owner_uuid.is_none());
        assert!(handles.find_one(select.clone()).await.unwrap().unwrap() == before);
        handles.delete_one(select).await.unwrap();
        let lookup = migration::lookup_finding(db, keys, &finding.id, None, true)
            .await
            .unwrap();
        assert_eq!(lookup.disposition, "source_absent_new_pass_required");
        assert!(server.received_requests().await.unwrap().is_empty());
    });
}

#[test]
fn handle_identity_deadlines_epoch_and_primary_rotation() {
    run_native_journey_test(
        "handle_identity_deadlines_epoch_and_primary_rotation",
        || async {
            let server = MockServer::start().await;
            let (state, _, receipt) = discord_fixture(&server.uri()).await;
            let db = &state.db;
            let keys = &state.encryption_keys;
            let before = one_handle(&state, &receipt).await;
            let handles = db.collection::<Document>(HANDLES);
            let receipts = db.collection::<Document>(NATIVE_RECEIPTS);
            let parent_id = doc! {"_id":&receipt.id};
            let parent = receipts.find_one(parent_id.clone()).await.unwrap().unwrap();
            let adapter =
                crate::services::channel_adapters::discord::DiscordAdapter::media_test_adapter(
                    &server.uri(),
                );
            for field in [
                "_id",
                "receipt_id",
                "binding_digest",
                "ciphertext_context_version",
                "expires_at",
                "created_at",
            ] {
                let mut altered = before.clone();
                match field {
                    "ciphertext_context_version" => {
                        altered.insert(field, 9_i32);
                    }
                    "expires_at" | "created_at" => {
                        let n = altered.get_datetime(field).unwrap().timestamp_millis();
                        altered.insert(field, bson::DateTime::from_millis(n - 1));
                    }
                    _ => {
                        altered.insert(field, uuid::Uuid::new_v4().to_string());
                    }
                }
                assert!(record::verify(db, keys, &altered).await.is_err(), "{field}");
            }
            let mut other = receipt.clone();
            other.id = uuid::Uuid::new_v4().to_string();
            other.sender.sender_id = "other-sender".into();
            receipts
                .insert_one(bson::to_document(&other).unwrap())
                .await
                .unwrap();
            let mut relinked = before.clone();
            relinked.insert("receipt_id", &other.id);
            relinked.insert("binding_digest", binding_digest(&other).unwrap());
            assert!(record::migrate(db, keys, &relinked).await.is_err());
            for field in [
                "profile_id",
                "manager_authority_epoch",
                "sender.sender_id",
                "configuration.credential_fingerprint",
                "content_commitment",
            ] {
                let value = if field == "manager_authority_epoch" {
                    bson::Bson::Int64(2)
                } else {
                    bson::Bson::String("c".repeat(64))
                };
                receipts
                    .update_one(parent_id.clone(), doc! {"$set":{field:value}})
                    .await
                    .unwrap();
                assert!(record::migrate(db, keys, &before).await.is_err(), "{field}");
                receipts
                    .replace_one(parent_id.clone(), &parent)
                    .await
                    .unwrap();
            }
            state
                .db
                .collection::<Document>(
                    crate::models::platform_bot_runtime_controls::COLLECTION_NAME,
                )
                .update_one(
                    doc! {"platform_config_id":&receipt.profile_id},
                    doc! {"$inc":{"manager_authority_epoch":1_i64}},
                )
                .await
                .unwrap();
            assert!(claim(&state, &receipt, &adapter).await.is_err());
            let mut config = crate::test_utils::test_app_config();
            config.encryption_key_previous = config.encryption_key.replace("22".repeat(32));
            config.allow_legacy_record_reads = false;
            let rotated = EncryptionKeys::from_config(&config);
            config.encryption_key_previous = None;
            let primary = EncryptionKeys::from_config(&config);
            assert!(record::migrate(db, &primary, &before).await.is_err());
            let select = doc! {"_id":before.get("_id").unwrap()};
            handles
                .update_one(select.clone(), doc! {"$set":{"retained_fixture":true}})
                .await
                .unwrap();
            assert!(!record::migrate(db, &rotated, &before).await.unwrap());
            let winning = handles.find_one(select.clone()).await.unwrap().unwrap();
            assert!(record::migrate(db, &rotated, &winning).await.unwrap());
            let current = handles.find_one(select).await.unwrap().unwrap();
            record::verify(db, &primary, &current).await.unwrap();
            assert!(current.get_bool("retained_fixture").unwrap());
            assert!(claim(&state, &receipt, &adapter).await.is_err());
            assert!(server.received_requests().await.unwrap().is_empty());
        },
    );
}

#[test]
fn retained_expired_interaction_uses_its_original_shorter_deadline() {
    run_native_journey_test(
        "retained_expired_interaction_uses_its_original_shorter_deadline",
        || async {
            let server = MockServer::start().await;
            let (state, _, mut receipt) = discord_fixture(&server.uri()).await;
            // Keep expired rows deterministically. The normal test fixture installs
            // both TTL indexes and MongoDB may purge either collection first.
            let db = connect_transaction_test_database("retained_expired_native_handle").await;
            let keys = &state.encryption_keys;
            let now = Utc::now();
            let interaction = now - Duration::minutes(30);
            let snowflake = (interaction.timestamp_millis() - 1_420_070_400_000) << 22;
            receipt.event_id = snowflake.to_string();
            receipt.message_id = receipt.event_id.clone();
            receipt.received_at = interaction + Duration::minutes(11);
            receipt.expires_at = receipt.received_at + Duration::minutes(10);
            receipt.content_commitment =
                crate::services::platform_entry_identity::content_commitment(
                    state.auth_device_hmac_key.as_slice(),
                    &receipt,
                    "/help",
                )
                .unwrap();
            let digest = binding_digest(&receipt).unwrap();
            db.collection::<NativeEntryReceipt>(NATIVE_RECEIPTS)
                .insert_one(&receipt)
                .await
                .unwrap();
            let bytes = HistoricalAad::native_reply(&digest)
                .unwrap()
                .seal(keys, br#"{"application_id":"100","token":"fixture-old"}"#)
                .await
                .unwrap();
            let expiry = interaction + Duration::minutes(14);
            let row = doc! {
                "_id": uuid::Uuid::new_v4().to_string(),
                "receipt_id": &receipt.id,
                "binding_digest": &digest,
                "created_at": bson::DateTime::from_chrono(receipt.received_at),
                "expires_at": bson::DateTime::from_chrono(expiry),
                "ciphertext": bytes.iter().map(|b|i32::from(*b)).collect::<Vec<_>>(),
            };
            let handles = db.collection::<Document>(HANDLES);
            handles.insert_one(&row).await.unwrap();
            let mut extended = row.clone();
            extended.insert(
                "expires_at",
                bson::DateTime::from_chrono(receipt.expires_at),
            );
            assert!(record::migrate(&db, keys, &extended).await.is_err());
            let mut strict = test_encryption_keys();
            strict.set_allow_legacy_record_reads(false);
            assert!(record::migrate(&db, &strict, &row).await.is_err());
            let mut wrong = row.clone();
            wrong.insert("binding_digest", "c".repeat(64));
            assert!(record::migrate(&db, keys, &wrong).await.is_err());
            assert!(record::migrate(&db, keys, &row).await.unwrap());
            let sealed = handles.find_one(doc! {}).await.unwrap().unwrap();
            assert_eq!(sealed.get("expires_at"), row.get("expires_at"));
            record::verify(&db, &strict, &sealed).await.unwrap();
            let model = bson::from_document(sealed).unwrap();
            assert!(record::check_live(&model, &receipt).is_err());
            assert!(server.received_requests().await.unwrap().is_empty());
        },
    );
}

#[test]
fn receipt_purge_while_sealing_leaves_the_surviving_handle_blocked() {
    use crate::crypto::{
        key_provider::{KeyProvider, WrappedKey},
        local_key_provider::LocalKeyProvider,
    };
    #[derive(Debug)]
    struct PausedWrap {
        local: LocalKeyProvider,
        entered: tokio::sync::Notify,
        resume: tokio::sync::Notify,
    }
    #[async_trait::async_trait]
    impl KeyProvider for PausedWrap {
        async fn wrap_dek(&self, key: &[u8]) -> AppResult<WrappedKey> {
            self.entered.notify_one();
            self.resume.notified().await;
            self.local.wrap_dek(key).await
        }
        async fn unwrap_dek(&self, key: &WrappedKey) -> AppResult<Zeroizing<Vec<u8>>> {
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
    run_native_journey_test(
        "receipt_purge_while_sealing_leaves_the_surviving_handle_blocked",
        || async {
            let server = MockServer::start().await;
            let mut legacy = test_encryption_keys();
            legacy.set_contextual_record_writes(false);
            let (state, _, receipt) = discord_fixture_with_keys(&server.uri(), legacy).await;
            let original = one_handle(&state, &receipt).await;
            let gate = Arc::new(PausedWrap {
                local: LocalKeyProvider::new([0x11; 32], None),
                entered: Default::default(),
                resume: Default::default(),
            });
            let keys = EncryptionKeys::with_provider(gate.clone());
            let purge = async {
                gate.entered.notified().await;
                state
                    .db
                    .collection::<Document>(NATIVE_RECEIPTS)
                    .delete_one(doc! {"_id": &receipt.id})
                    .await
                    .unwrap();
                gate.resume.notify_one();
            };
            let (rewritten, ()) = tokio::join!(record::migrate(&state.db, &keys, &original), purge);
            assert!(!rewritten.unwrap());
            assert!(one_handle(&state, &receipt).await == original);
            let Err(crate::errors::AppError::Conflict(reason)) =
                record::verify(&state.db, &keys, &original).await
            else {
                panic!("missing receipt finding")
            };
            assert_eq!(reason, record::RECEIPT_MISSING);
            assert!(server.received_requests().await.unwrap().is_empty());
        },
    );
}
