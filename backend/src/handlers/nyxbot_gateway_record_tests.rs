use super::*;
use crate::{
    crypto::{
        aes::EncryptionKeys,
        record_field::{self, RecordContext},
    },
    models::nyxbot_gateway_response::{COLLECTION_NAME as RESPONSES, GatewayResponse},
    services::{encrypted_record_migration as migration, nyxbot_response_recovery as recovery},
};
use bson::Document;

async fn plain(keys: &EncryptionKeys, row: &GatewayResponse) -> Vec<u8> {
    let field = [
        "payload_ciphertext",
        &row.channel_id,
        &row.binding_id,
        &row.response_id,
    ];
    let context = RecordContext::new(RESPONSES, &row.id, &row.user_id, &field).unwrap();
    record_field::decrypt_context(
        keys,
        &context,
        row.payload_context_version,
        &row.payload_ciphertext,
    )
    .await
    .unwrap()
    .to_vec()
}

async fn capture(keys: &EncryptionKeys, row: &GatewayResponse) -> Document {
    let raw = plain(keys, row).await;
    let body = recovery::payload(keys, row).await.unwrap();
    assert_eq!(raw, serde_json::to_vec(&body).unwrap());
    assert_eq!(body.frames.last().unwrap()["response"], body.snapshot);
    for (n, frame) in body.frames.iter().enumerate() {
        assert_eq!(frame["sequence_number"], n);
    }
    doc! {"row": bson::to_document(row).unwrap(), "bytes": bson::Binary {
        subtype:bson::spec::BinarySubtype::Generic, bytes:raw,
    }}
}

async fn producer_fixtures(keys: EncryptionKeys) -> Document {
    let (mut state, _, server) = setup("native_record_producer").await;
    state.encryption_keys = Arc::new(keys);
    let (native, key) = native_tests::native_channel(&state).await;
    let response = respond(
        &state,
        &key,
        &native_tests::native_event(&native, "500", "Hello"),
        "native-gateway-record",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let id = response.headers()["x-cma-response-id"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(body_text(response).await.contains("response.completed"));
    let native = recovery::load(&state.db, &native, &id)
        .await
        .unwrap()
        .unwrap();
    let mut rows = doc! {"native":capture(&state.encryption_keys,&native).await};
    server.abort();

    let (mut relay, _, server) = setup("relay_record_producer").await;
    relay.encryption_keys = state.encryption_keys.clone();
    let (channel, _, gateway, request) = recovery_channel(&relay, false).await;
    let mut event = admission(&channel.id, "relay-recovery-event");
    event.status = "running".into();
    event.relay_message_id = Some(event.event_id.clone());
    relay
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .insert_one(&event)
        .await
        .unwrap();
    let pending =
        relay_responses::prepare(&relay, &channel, &event, &request, "resp_relay_fixture")
            .await
            .unwrap()
            .stored
            .unwrap();
    rows.insert(
        "relay_pending",
        capture(&relay.encryption_keys, &pending).await,
    );
    // Give each revision its own fixture row when the migration driver imports them.
    recovery::finish(
        &relay.db,
        &relay.encryption_keys,
        &pending,
        Some("第一行\n🗝 second line"),
        false,
    )
    .await
    .unwrap();
    let terminal = recovery::load(&relay.db, &channel, &pending.response_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(terminal.revision, pending.revision + 1);
    let body = recovery::payload(&relay.encryption_keys, &terminal)
        .await
        .unwrap();
    assert_eq!(
        body.snapshot["output"][0]["content"][0]["text"],
        "第一行\n🗝 second line"
    );
    rows.insert(
        "relay_terminal",
        capture(&relay.encryption_keys, &terminal).await,
    );
    drop(gateway);
    server.abort();
    rows
}

#[tokio::test]
async fn mixed_native_and_relay_gateway_payloads() {
    let Ok(step) = std::env::var("NYXID_RECORD_FIXTURE_STEP") else {
        return;
    };
    let file = std::env::var("NYXID_RECORD_FIXTURE_FILE").unwrap();
    let mut keys = crate::test_utils::test_encryption_keys();
    keys.set_contextual_record_writes(
        std::env::var("CONTEXTUAL_RECORD_WRITES").as_deref() == Ok("true"),
    );
    keys.set_allow_legacy_record_reads(
        std::env::var("ALLOW_LEGACY_RECORD_READS").as_deref() != Ok("false"),
    );
    if step == "write" || step == "write-active" {
        let rows = Box::pin(producer_fixtures(keys)).await;
        for item in rows.values() {
            let source = item.as_document().unwrap().get_document("row").unwrap();
            assert_eq!(
                source.get_i32("payload_context_version").ok(),
                (step == "write-active").then_some(3)
            );
        }
        std::fs::write(file, bson::to_vec(&rows).unwrap()).unwrap();
        return;
    }
    let mut rows: Document = bson::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    for (label, item) in rows.iter_mut() {
        let fixture = item.as_document_mut().unwrap();
        let source = fixture.get_document("row").unwrap();
        if step == "gateway-migrate" {
            // Isolate the retained revisions of the same real response.
            let db = crate::test_utils::connect_transaction_test_database(label).await;
            let responses = db.collection::<Document>(RESPONSES);
            responses.insert_one(source).await.unwrap();
            let run = migration::start(&db, &keys, migration::Mode::Migrate, None)
                .await
                .unwrap();
            loop {
                let status = migration::status(&db, &keys).await.unwrap().unwrap();
                if status.phase == "observed" {
                    assert_eq!((status.rewritten, status.unresolved), (1, 0));
                    assert_eq!(
                        status
                            .field_outcomes
                            .iter()
                            .find(|s| s.collection == RESPONSES)
                            .unwrap()
                            .authenticated_current,
                        1
                    );
                    break;
                }
                Box::pin(migration::advance(&db, &keys, &run))
                    .await
                    .unwrap();
            }
            fixture.insert("row", responses.find_one(doc! {}).await.unwrap().unwrap());
            db.drop().await.unwrap();
        } else {
            let row: GatewayResponse = bson::from_document(source.clone()).unwrap();
            let payload = recovery::payload(&keys, &row).await;
            if step == "gateway-reject" {
                assert!(payload.is_err());
                continue;
            }
            assert_eq!(step, "gateway-read");
            let expected = fixture.get_binary_generic("bytes").unwrap();
            assert_eq!(&plain(&keys, &row).await, expected);
            assert_eq!(&serde_json::to_vec(&payload.unwrap()).unwrap(), expected);
        }
    }
    if step == "gateway-migrate" {
        std::fs::write(file, bson::to_vec(&rows).unwrap()).unwrap();
    }
}

// Candidate storage tests
#[tokio::test]
async fn gateway_record_rewrap_preserves_expired_bytes_and_loses_to_changed_source() {
    use recovery::storage;
    let mut old = crate::test_utils::test_encryption_keys();
    old.set_contextual_record_writes(false);
    let fixtures = Box::pin(producer_fixtures(old)).await;
    let fixture = fixtures.get_document("relay_terminal").unwrap();
    let mut source = fixture.get_document("row").unwrap().clone();
    let filter = doc! {"_id":source.get_str("_id").unwrap()};
    let now = Utc::now();
    source.insert(
        "created_at",
        bson::DateTime::from_chrono(now - ChronoDuration::hours(26)),
    );
    source.insert(
        "expires_at",
        bson::DateTime::from_chrono(now - ChronoDuration::hours(2)),
    );
    // Retained historical JSON may include whitespace. A serde round trip loses it.
    let mut bytes = vec![b' '];
    bytes.extend(fixture.get_binary_generic("bytes").unwrap());
    let keys = crate::test_utils::test_encryption_keys();
    let encrypted = keys.encrypt(&bytes).await.unwrap();
    source.insert(
        "payload_ciphertext",
        encrypted.into_iter().map(i32::from).collect::<Vec<_>>(),
    );
    let db = crate::test_utils::connect_transaction_test_database("gateway_rewrap").await;
    let rows = db.collection::<Document>(RESPONSES);
    rows.insert_one(&source).await.unwrap();
    for changed in [
        doc! {"revision":42_i64},
        doc! {"terminal":false},
        doc! {"answer_delivery":"unconfirmed"},
        doc! {"event_key":"new-event"},
        doc! {"request_hash":"new-request"},
        doc! {"conversation_id":"new-conversation"},
        doc! {"expires_at":bson::DateTime::now()},
        doc! {"user_id":"other-owner"},
        doc! {"checked_at":bson::DateTime::now()},
        doc! {"payload_context_version":bson::Bson::Null},
    ] {
        let mut winner = source.clone();
        winner.extend(changed);
        rows.replace_one(filter.clone(), &winner).await.unwrap();
        assert!(!storage::migrate(&db, &keys, &source).await.unwrap());
        assert_eq!(
            rows.find_one(filter.clone()).await.unwrap().unwrap(),
            winner
        );
    }
    rows.replace_one(filter.clone(), &source).await.unwrap();
    assert!(storage::migrate(&db, &keys, &source).await.unwrap());
    let current = rows.find_one(filter.clone()).await.unwrap().unwrap();
    storage::verify(&keys, &current).await.unwrap();
    assert!(!storage::migrate(&db, &keys, &current).await.unwrap());
    let row = bson::from_document(current.clone()).unwrap();
    assert_eq!(plain(&keys, &row).await, bytes);
    let mut metadata = current.clone();
    assert_eq!(
        metadata.remove("payload_context_version"),
        Some(3_i32.into())
    );
    metadata.insert(
        "payload_ciphertext",
        source.get("payload_ciphertext").unwrap(),
    );
    assert_eq!(metadata, source);
    for path in [
        "_id",
        "response_id",
        "user_id",
        "channel_id",
        "binding_id",
        "partition",
    ] {
        let mut wrong = current.clone();
        wrong.insert(path, "different");
        assert!(storage::verify(&keys, &wrong).await.is_err());
        assert!(storage::migrate(&db, &keys, &wrong).await.is_err());
    }
    let mut invalid = current.clone();
    invalid.insert("payload_context_version", 42);
    assert!(storage::migrate(&db, &keys, &invalid).await.is_err());
    let mut body = recovery::payload(&keys, &row).await.unwrap();
    body.frames.swap(0, 1);
    let mut wrong = row.clone();
    storage::install(&keys, &mut wrong, &body).await.unwrap();
    assert!(
        storage::verify(&keys, &bson::to_document(&wrong).unwrap())
            .await
            .is_err()
    );
    let mut config = crate::test_utils::test_app_config();
    config.encryption_key_previous = config.encryption_key.replace("33".repeat(32));
    let rotated = EncryptionKeys::from_config(&config);
    assert!(storage::migrate(&db, &rotated, &current).await.unwrap());
    let rotated_row = rows.find_one(filter.clone()).await.unwrap().unwrap();
    storage::verify(&rotated, &rotated_row).await.unwrap();
    assert_eq!(
        plain(&rotated, &bson::from_document(rotated_row).unwrap()).await,
        bytes
    );
    rows.delete_one(filter.clone()).await.unwrap();
    assert!(!storage::migrate(&db, &keys, &source).await.unwrap());
    assert!(rows.find_one(filter).await.unwrap().is_none());
    db.drop().await.unwrap();
}

#[tokio::test]
async fn gateway_record_finish_and_migration_win_in_either_order_without_repeating_frames() {
    use recovery::storage;
    let mut old = crate::test_utils::test_encryption_keys();
    old.set_contextual_record_writes(false);
    let fixtures = Box::pin(producer_fixtures(old)).await;
    let source = fixtures
        .get_document("relay_pending")
        .unwrap()
        .get_document("row")
        .unwrap();
    let pending: GatewayResponse = bson::from_document(source.clone()).unwrap();
    let db = crate::test_utils::connect_transaction_test_database("gateway_replay_race").await;
    let rows = db.collection::<Document>(RESPONSES);
    let keys = crate::test_utils::test_encryption_keys();
    let filter = doc! {"_id":&pending.id};
    for migration_first in [false, true] {
        rows.replace_one(filter.clone(), source)
            .upsert(true)
            .await
            .unwrap();
        if migration_first {
            assert!(storage::migrate(&db, &keys, source).await.unwrap());
        }
        recovery::finish(&db, &keys, &pending, Some("first"), false)
            .await
            .unwrap();
        assert!(!storage::migrate(&db, &keys, source).await.unwrap());
        let winner = rows.find_one(filter.clone()).await.unwrap().unwrap();
        recovery::finish(&db, &keys, &pending, Some("second"), true)
            .await
            .unwrap();
        assert_eq!(
            rows.find_one(filter.clone()).await.unwrap().unwrap(),
            winner
        );
        storage::verify(&keys, &winner).await.unwrap();
        let row: GatewayResponse = bson::from_document(winner).unwrap();
        assert_eq!(row.revision, pending.revision + 1);
        assert!(row.terminal);
        let mut expected = recovery::payload(&keys, &pending).await.unwrap();
        expected.finish(Some("first"), false);
        let mut actual = recovery::payload(&keys, &row).await.unwrap();
        assert_eq!(
            actual.encoded().unwrap().as_slice(),
            expected.encoded().unwrap().as_slice()
        );
        recovery::suppress_output(&mut actual);
        recovery::suppress_output(&mut expected);
        assert_eq!(
            actual.encoded().unwrap().as_slice(),
            expected.encoded().unwrap().as_slice()
        );
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn gateway_record_migrates_a_retained_payload_larger_than_half_a_bson_document() {
    use recovery::storage;
    let db = crate::test_utils::connect_transaction_test_database("gateway_large_record").await;
    let keys = crate::test_utils::test_encryption_keys();
    let mut body = recovery::Payload::new(
        "response",
        &recovery::conversation_identity("channel", "binding", "partition"),
        Default::default(),
    );
    body.finish_text(Some(&"a".repeat(3 * 1024 * 1024)), false);
    let bytes = body.encoded().unwrap();
    assert!(bytes.len() > 8 * 1024 * 1024);
    let now = bson::DateTime::now();
    let source = doc! {
        "_id":"large","response_id":"response","channel_id":"channel","binding_id":"binding",
        "user_id":"owner","partition":"partition","event_key":"event","request_hash":"hash",
        "revision":1_i64,"terminal":true,"created_at":now,"expires_at":now,
        "payload_ciphertext":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:keys.encrypt(&bytes).await.unwrap()},
    };
    let rows = db.collection::<Document>(RESPONSES);
    rows.insert_one(&source).await.unwrap();
    assert!(storage::migrate(&db, &keys, &source).await.unwrap());
    let saved = rows.find_one(doc! {"_id":"large"}).await.unwrap().unwrap();
    storage::verify(&keys, &saved).await.unwrap();
    assert_eq!(
        plain(&keys, &bson::from_document(saved).unwrap()).await,
        *bytes
    );
    db.drop().await.unwrap();
}
