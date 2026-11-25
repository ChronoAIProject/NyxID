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
    assert!(!gateway.contains_key("payload_context_version"));
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
    assert!(
        !event
            .get_document("response_reply")
            .unwrap()
            .contains_key("context_version")
    );
    let notice = response_read::load(state, &channel, &id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        notice.payload.snapshot["output"][0]["content"][0]["text"],
        "stored notice"
    );
    doc! {
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
        let bytes = keys.decrypt(ciphertext).await.unwrap();
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
async fn gateway_replay_refresh_and_finish_replace_contextual_fixtures_with_shipped_encoding() {
    let (state, _, server) = setup("gateway_replay_replacement").await;
    let mut rows = Box::pin(written_rows(&state)).await;
    let source = rows.get_document_mut("gateway").unwrap();
    let row: GatewayResponse = bson::from_document(source.clone()).unwrap();
    let payload =
        recovery::Payload::new(&row.response_id, "fixture-conversation", Default::default());
    let sealed = payload.seal(&state.encryption_keys).await.unwrap();
    source.insert(
        "payload_ciphertext",
        bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: sealed,
        },
    );
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
    for finish in [false, true] {
        responses
            .replace_one(doc! {"_id":&current.id}, gateway)
            .await
            .unwrap();
        if finish {
            recovery::finish(
                &state.db,
                &state.encryption_keys,
                &current,
                Some("completed"),
                false,
            )
            .await
            .unwrap();
        } else {
            recovery::refresh(
                &state.db,
                &state.encryption_keys,
                &current,
                std::future::ready,
            )
            .await
            .unwrap();
        }
        let updated = responses
            .find_one(doc! {"_id":&current.id})
            .await
            .unwrap()
            .unwrap();
        assert!(!updated.contains_key("payload_context_version"));
        let row: GatewayResponse = bson::from_document(updated).unwrap();
        assert_eq!(row.revision, current.revision + 1);
        assert_eq!(row.expires_at, current.expires_at);
        assert_eq!(row.terminal, finish);
        let decoded = recovery::payload(&state.encryption_keys, &row)
            .await
            .unwrap();
        assert_eq!(
            decoded.snapshot["status"],
            if finish { "completed" } else { "in_progress" }
        );
    }
    server.abort();
}
