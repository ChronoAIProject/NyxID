use super::*;
use crate::{
    crypto::{aes::EncryptionKeys, record_field::RecordContext},
    models::nyxbot_gateway_response::{COLLECTION_NAME as RESPONSES, GatewayResponse},
    services::{nyxbot_response_read as read, nyxbot_response_recovery as recovery},
};
use bson::Document;

async fn written_rows(state: &AppState) -> Document {
    let (native, key) = native_tests::native_channel(state).await;
    let request = native_tests::native_event(&native, "500", "Hello");
    let response = respond(state, &key, &request, "mixed-native").await;
    assert_eq!(response.status(), StatusCode::OK);
    let id = response.headers()["x-cma-response-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let frames = body_text(response).await;
    assert!(frames.contains("response.completed"));
    let gateway = state
        .db
        .collection::<Document>(RESPONSES)
        .find_one(doc! {"response_id":&id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        gateway.get_i32("payload_context_version").ok(),
        state
            .encryption_keys
            .contextual_record_writes()
            .then_some(3)
    );
    let stored: GatewayResponse = bson::from_document(gateway.clone()).unwrap();
    let expected = recovery::payload(&state.encryption_keys, &stored)
        .await
        .unwrap();

    let (channel, _) = channel(state, "gateway").await;
    let mut event = admission(&channel.id, "mixed-notice");
    event.status = "running".into();
    event.relay_message_id = Some("original-message".into());
    let mapping = read::identity(&channel, &event, read::command(&id, &json!({})));
    let created = recovery::Payload::new(&id, &mapping.conversation_id, Default::default())
        .frames
        .remove(0);
    let mut source = bson::to_document(&event).unwrap();
    source.insert("response_read", bson::to_document(&mapping).unwrap());
    let events = state.db.collection::<Document>(EVENTS);
    events.insert_one(source).await.unwrap();
    let response = response_read::reply(
        state.clone(),
        &channel,
        event.id.clone(),
        id.clone(),
        created,
        "stored notice".into(),
        false,
    )
    .await
    .unwrap();
    assert!(body_text(response).await.contains("stored notice"));
    let event = events
        .find_one(doc! {"_id":&event.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        event
            .get_document("response_reply")
            .unwrap()
            .contains_key("context_version"),
        state.encryption_keys.contextual_record_writes()
    );
    let notice = response_read::load(state, &channel, &id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        notice.payload.snapshot["output"][0]["content"][0]["text"],
        "stored notice"
    );
    let native_event = events
        .find_one(doc! {"_id": &stored.event_key})
        .await
        .unwrap()
        .unwrap();
    doc! {
        "native_event": native_event,
        "expected_context": bson::to_bson(&request["event_context"]).unwrap(),
        "channel": bson::to_document(&channel).unwrap(),
        "event": event,
        "gateway": gateway,
        "expected_gateway": bson::to_document(&expected).unwrap(),
        "expected_reply": bson::to_document(&notice.payload).unwrap(),
    }
}

async fn contextualize(keys: &EncryptionKeys, rows: &mut Document) {
    for event in [true, false] {
        let collection = if event { EVENTS } else { RESPONSES };
        let source = rows
            .get_document_mut(if event { "event" } else { "gateway" })
            .unwrap();
        if event
            && source
                .get_document("response_reply")
                .unwrap()
                .get_i32("context_version")
                == Ok(3)
        {
            continue;
        }
        if !event && source.get_i32("payload_context_version") == Ok(3) {
            continue;
        }
        let id = source.get_str("_id").unwrap();
        let owner = source.get_str("user_id").unwrap();
        let channel = source.get_str("channel_id").unwrap();
        let (parts, ciphertext) = if event {
            let reply = source.get_document("response_reply").unwrap();
            (
                vec![
                    "response_reply.ciphertext",
                    channel,
                    reply.get_str("response_id").unwrap(),
                ],
                reply.get_binary_generic("ciphertext").unwrap(),
            )
        } else {
            (
                vec![
                    "payload_ciphertext",
                    channel,
                    source.get_str("binding_id").unwrap(),
                    source.get_str("response_id").unwrap(),
                ],
                source.get_binary_generic("payload_ciphertext").unwrap(),
            )
        };
        let context = RecordContext::new(collection, id, owner, &parts).unwrap();
        let bytes = crate::crypto::record_field::decrypt_context(keys, &context, None, ciphertext)
            .await
            .unwrap();
        let sealed = bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: keys.encrypt_record(&bytes, &context).await.unwrap(),
        };
        if event {
            let reply = source.get_document_mut("response_reply").unwrap();
            reply.insert("ciphertext", sealed);
            reply.insert("context_version", 3);
        } else {
            source.insert("payload_ciphertext", sealed);
            source.insert("payload_context_version", 3);
        }
    }
}

async fn read_rows(state: &AppState, rows: &Document, reject: bool) {
    let gateway: GatewayResponse =
        bson::from_document(rows.get_document("gateway").unwrap().clone()).unwrap();
    let channel: NyxbotChannel =
        bson::from_document(rows.get_document("channel").unwrap().clone()).unwrap();
    let event = rows.get_document("event").unwrap();
    state
        .db
        .collection::<Document>(EVENTS)
        .replace_one(doc! {"_id":event.get_str("_id").unwrap()}, event)
        .upsert(true)
        .await
        .unwrap();
    let body = recovery::payload(&state.encryption_keys, &gateway).await;
    let reply = response_read::load(state, &channel, &gateway.response_id).await;
    if reject {
        assert!(body.is_err());
        assert!(reply.is_err());
    } else {
        assert_eq!(
            &bson::to_document(&body.unwrap()).unwrap(),
            rows.get_document("expected_gateway").unwrap()
        );
        assert_eq!(
            &bson::to_document(&reply.unwrap().unwrap().payload).unwrap(),
            rows.get_document("expected_reply").unwrap()
        );
    }
}

#[tokio::test]
async fn mixed_event_and_gateway_replay() {
    let Ok(step) = std::env::var("NYXID_RECORD_FIXTURE_STEP") else {
        return;
    };
    let file = std::env::var("NYXID_RECORD_FIXTURE_FILE").unwrap();
    let (mut state, _, server) = setup("mixed_event_replay").await;
    let mut keys = crate::test_utils::test_encryption_keys();
    keys.set_contextual_record_writes(
        std::env::var("CONTEXTUAL_RECORD_WRITES").as_deref() == Ok("true"),
    );
    keys.set_allow_legacy_record_reads(
        std::env::var("ALLOW_LEGACY_RECORD_READS").as_deref() != Ok("false"),
    );
    state.encryption_keys = Arc::new(keys);
    match step.as_str() {
        "event-read" | "event-reject" => {
            let rows: Document = bson::from_slice(&std::fs::read(file).unwrap()).unwrap();
            let native: NyxbotEvent =
                bson::from_document(rows.get_document("native_event").unwrap().clone()).unwrap();
            let plain = event_record::event_context(
                &state.encryption_keys,
                &native,
                native.event_context_ciphertext.as_deref().unwrap(),
            )
            .await;
            let channel =
                bson::from_document(rows.get_document("channel").unwrap().clone()).unwrap();
            let event = rows.get_document("event").unwrap();
            state
                .db
                .collection::<Document>(EVENTS)
                .insert_one(event)
                .await
                .unwrap();
            let id = event
                .get_document("response_reply")
                .unwrap()
                .get_str("response_id")
                .unwrap();
            let reply = response_read::load(&state, &channel, id).await;
            if step == "event-reject" {
                assert!(plain.is_err() && reply.is_err());
            } else {
                let value: Value = serde_json::from_slice(&plain.unwrap()).unwrap();
                assert_eq!(bson::to_bson(&value).unwrap(), rows["expected_context"]);
                assert_eq!(
                    bson::to_document(&reply.unwrap().unwrap().payload).unwrap(),
                    *rows.get_document("expected_reply").unwrap()
                );
            }
        }
        "event-migrate" => {
            let mut rows: Document = bson::from_slice(&std::fs::read(&file).unwrap()).unwrap();
            let events = state.db.collection::<Document>(EVENTS);
            for key in ["native_event", "event"] {
                let source = rows.get_document(key).unwrap();
                events.insert_one(source).await.unwrap();
                assert!(
                    event_record::migrate(&state.db, &state.encryption_keys, source)
                        .await
                        .unwrap()
                );
                let current = events
                    .find_one(doc! {"_id":source.get_str("_id").unwrap()})
                    .await
                    .unwrap()
                    .unwrap();
                event_record::verify(&state.encryption_keys, &current)
                    .await
                    .unwrap();
                rows.insert(key, current);
            }
            std::fs::write(file, bson::to_vec(&rows).unwrap()).unwrap();
        }
        "write" | "contextual-fixture" => {
            let mut rows = Box::pin(written_rows(&state)).await;
            if step == "contextual-fixture" {
                contextualize(&state.encryption_keys, &mut rows).await;
            }
            std::fs::write(file, bson::to_vec(&rows).unwrap()).unwrap();
        }
        "read" | "reject" => {
            let rows = bson::from_slice(&std::fs::read(file).unwrap()).unwrap();
            Box::pin(read_rows(&state, &rows, step == "reject")).await;
        }
        _ => panic!("unexpected fixture step"),
    }
    server.abort();
}

#[tokio::test]
async fn swapped_event_and_gateway_replay_payloads_are_rejected_in_both_collections() {
    let (state, _, server) = setup("replay_collection_swap").await;
    let mut rows = Box::pin(written_rows(&state)).await;
    contextualize(&state.encryption_keys, &mut rows).await;
    Box::pin(read_rows(&state, &rows, false)).await;
    let event = rows
        .get_document_mut("event")
        .unwrap()
        .get_document_mut("response_reply")
        .unwrap();
    let notice = event.remove("ciphertext").unwrap();
    let gateway = rows.get_document_mut("gateway").unwrap();
    let replay = gateway.insert("payload_ciphertext", notice).unwrap();
    rows.get_document_mut("event")
        .unwrap()
        .get_document_mut("response_reply")
        .unwrap()
        .insert("ciphertext", replay);
    Box::pin(read_rows(&state, &rows, true)).await;
    server.abort();
}

#[tokio::test]
async fn gateway_replay_refresh_and_finish_select_the_writer_encoding() {
    let (state, _, server) = setup("gateway_replay_replacement").await;
    let mut rows = Box::pin(written_rows(&state)).await;
    let source = rows.get_document_mut("gateway").unwrap();
    let mut row: GatewayResponse = bson::from_document(source.clone()).unwrap();
    let payload =
        recovery::Payload::new(&row.response_id, "fixture-conversation", Default::default());
    let mut compatible = crate::test_utils::test_encryption_keys();
    compatible.set_contextual_record_writes(false);
    recovery::storage::install(&compatible, &mut row, &payload)
        .await
        .unwrap();
    assert_eq!(row.payload_context_version, None);
    source.insert(
        "payload_ciphertext",
        bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: row.payload_ciphertext,
        },
    );
    source.remove("payload_context_version");
    source.insert("terminal", false);
    contextualize(&state.encryption_keys, &mut rows).await;
    let gateway = rows.get_document("gateway").unwrap();
    let current: GatewayResponse = bson::from_document(gateway.clone()).unwrap();
    let events = state.db.collection::<Document>(EVENTS);
    events
        .update_one(
            doc! {"_id":&current.event_key},
            doc! {"$set":{"conversation_id":"waiting-conversation","turn_id":"waiting-turn"}},
        )
        .await
        .unwrap();
    let responses = state.db.collection::<Document>(RESPONSES);
    for (writes, finish) in [(false, false), (false, true), (true, false), (true, true)] {
        let mut keys = crate::test_utils::test_encryption_keys();
        keys.set_contextual_record_writes(writes);
        responses
            .replace_one(doc! {"_id":&current.id}, gateway)
            .await
            .unwrap();
        if finish {
            recovery::finish(&state.db, &keys, &current, Some("completed"), false)
                .await
                .unwrap();
        } else {
            recovery::refresh(&state.db, &keys, &current, std::future::ready)
                .await
                .unwrap();
        }
        let updated = responses
            .find_one(doc! {"_id":&current.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            updated.get_i32("payload_context_version").ok(),
            writes.then_some(3)
        );
        let row: GatewayResponse = bson::from_document(updated).unwrap();
        assert_eq!(row.revision, current.revision + 1);
        assert_eq!(row.expires_at, current.expires_at);
        assert_eq!(row.terminal, finish);
        let decoded = recovery::payload(&keys, &row).await.unwrap();
        assert_eq!(
            decoded.snapshot["status"],
            if finish { "completed" } else { "in_progress" }
        );
    }
    server.abort();
}

#[tokio::test]
async fn event_record_seven_day_media_and_nested_claims_survive_exact_source_migration() {
    use crate::services::nyxbot_event_record as record;
    let (mut state, _, server) = setup("event_record_retained_media").await;
    let mut compatible = crate::test_utils::test_encryption_keys();
    compatible.set_contextual_record_writes(false);
    state.encryption_keys = Arc::new(compatible);
    let fixtures = Box::pin(written_rows(&state)).await;
    let mut source = fixtures.get_document("event").unwrap().clone();
    let event_id = source.get_str("event_id").unwrap();
    let partition = source.get_str("partition").unwrap();
    let channel = source.get_str("channel_id").unwrap();
    let context = json!({"activity":{"event_id":event_id},"conversation_id":partition});
    let guest = json!({
        "origin": {"nyxbot_channel_id":channel,"partition":partition,"platform":"telegram"},
        "conversation":"queued-chat", "agent":"guest-agent", "profile":"model", "text":"guest question"
    });
    let keys = &state.encryption_keys;
    let id = source.get_str("_id").unwrap();
    let owner = source.get_str("user_id").unwrap();
    let encrypted_context = record::seal(
        keys,
        id,
        owner,
        &[record::CONTEXT, channel],
        &serde_json::to_vec(&context).unwrap(),
    )
    .await
    .unwrap();
    let encrypted_target = record::seal(
        keys,
        id,
        owner,
        &[record::TARGET, channel],
        b"original-target",
    )
    .await
    .unwrap();
    let encrypted_guest = record::seal(
        keys,
        id,
        owner,
        &[record::GUEST, channel],
        &serde_json::to_vec(&guest).unwrap(),
    )
    .await
    .unwrap();
    let created = Utc::now() - ChronoDuration::days(6);
    let id = source.get_str("_id").unwrap().to_owned();
    let binary = |bytes| bson::Binary {
        subtype: bson::spec::BinarySubtype::Generic,
        bytes,
    };
    source.extend(doc! {
        "created_at":bson::DateTime::from_chrono(created),
        "expires_at":bson::DateTime::from_chrono(created + ChronoDuration::days(7)),
        "media":{"interrupted":false, "source_message_id":"original-media"},
        "guest_setup_conversation_id":"queued-chat",
        "guest_setup_payload":binary(encrypted_guest),
        "event_context_ciphertext":encrypted_context.iter().copied().map(i32::from).collect::<Vec<_>>(),
        "delivery":{
            "state":"pending",
            "target_ciphertext":binary(encrypted_target),
            "target_expires_at":bson::DateTime::from_chrono(created + ChronoDuration::hours(1)),
        },
    });
    let rows = state.db.collection::<Document>(EVENTS);
    let filter = doc! {"_id":&id};
    let active = crate::test_utils::test_encryption_keys();
    assert!(record::migrate(&state.db, keys, &source).await.is_err());
    for update in [
        doc! {"$set":{"delivery.claim_id":"competing-sender"}},
        doc! {"$set":{"guest_setup_claim":"restarted-guest"}},
        doc! {"$set":{"media.interrupted":true}},
        doc! {"$set":{"delivery.target_expires_at":bson::DateTime::now()}},
        doc! {"$unset":{"response_read":"","response_reply":""}},
        doc! {"$unset":{"guest_setup_payload":""}},
        doc! {"$set":{"user_id":"new-owner"}},
        doc! {"$set":{"event_context_ciphertext":binary(encrypted_context.clone())}},
    ] {
        rows.replace_one(filter.clone(), &source).await.unwrap();
        rows.update_one(filter.clone(), update).await.unwrap();
        let winner = rows.find_one(filter.clone()).await.unwrap().unwrap();
        assert!(!record::migrate(&state.db, &active, &source).await.unwrap());
        assert_eq!(
            rows.find_one(filter.clone()).await.unwrap().unwrap(),
            winner
        );
    }
    rows.replace_one(filter.clone(), &source).await.unwrap();
    assert!(record::migrate(&state.db, &active, &source).await.unwrap());
    let after = rows.find_one(filter.clone()).await.unwrap().unwrap();
    record::verify(&active, &after).await.unwrap();
    assert!(!record::migrate(&state.db, &active, &after).await.unwrap());
    let mut metadata = after.clone();
    for (parent, data, version) in [
        (None, "event_context_ciphertext", "event_context_version"),
        (None, "guest_setup_payload", "guest_setup_context_version"),
        (
            Some("delivery"),
            "target_ciphertext",
            "target_context_version",
        ),
        (Some("response_reply"), "ciphertext", "context_version"),
    ] {
        let original = parent.map_or(&source, |p| source.get_document(p).unwrap());
        let container = match parent {
            Some(p) => metadata.get_document_mut(p).unwrap(),
            None => &mut metadata,
        };
        assert_eq!(container.remove(version), Some(bson::Bson::Int32(3)));
        container.insert(data, original.get(data).unwrap());
    }
    assert_eq!(metadata, source);
    let mut config = crate::test_utils::test_app_config();
    config.encryption_key_previous = config.encryption_key.replace("33".repeat(32));
    let rotated = EncryptionKeys::from_config(&config);
    assert!(record::verify(&rotated, &after).await.is_err());
    assert!(record::migrate(&state.db, &rotated, &after).await.unwrap());
    let rewrapped = rows.find_one(filter).await.unwrap().unwrap();
    record::verify(&rotated, &rewrapped).await.unwrap();
    assert!(record::verify(&active, &rewrapped).await.is_err());
    for update in [
        doc! {"channel_id":"other-channel"},
        doc! {"user_id":"other-owner"},
        doc! {"_id":"same-owner-other-event"},
        doc! {"guest_setup_conversation_id":"other-guest"},
        doc! {"event_context_version":42},
        doc! {"event_context_ciphertext":bson::Bson::Null},
        doc! {"event_context_ciphertext":rewrapped.get_binary_generic("guest_setup_payload").unwrap().clone().into_iter().map(i32::from).collect::<Vec<_>>()},
    ] {
        let mut broken = rewrapped.clone();
        broken.extend(update);
        assert!(record::verify(&rotated, &broken).await.is_err());
        assert!(record::migrate(&state.db, &rotated, &broken).await.is_err());
    }
    let mut swapped = rewrapped.clone();
    swapped
        .get_document_mut("response_reply")
        .unwrap()
        .insert("response_id", "other-response");
    assert!(record::verify(&rotated, &swapped).await.is_err());
    record::verify(&active, &doc! {"_id":"retired","delivery":null})
        .await
        .unwrap();
    server.abort();
}
