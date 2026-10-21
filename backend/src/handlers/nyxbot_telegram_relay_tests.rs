use super::*;
use crate::services::{channel_platform::PlatformAdapter, channel_relay_service as relay};
use futures::TryStreamExt;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::*};

struct TelegramOrigin(String);
impl Drop for TelegramOrigin {
    fn drop(&mut self) {
        crate::services::telegram_input::TEST_API_BASES
            .lock()
            .unwrap()
            .remove(&self.0);
    }
}

#[tokio::test]
async fn gateway_file_failure_or_disabled_sharing_keeps_the_owner_text_turn() {
    for enabled in [false, true] {
        let (state, calls, model) = setup("telegram_file_degrade").await;
        let (row, agent_key) = channel(&state, "gateway").await;
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{"owner_sender_ids":["7"]}},
            )
            .await
            .unwrap();
        let token = state
            .encryption_keys
            .encrypt(b"123:quoted-relay-test-secret")
            .await
            .unwrap();
        state.db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
            .update_one(doc!{"_id":&row.channel_bot_id},doc!{"$set":{"bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:token}}}).await.unwrap();
        crate::services::feature_flag_service::set_platform_override(
            &state.db,
            crate::services::feature_flag_service::ASSISTANT_CHANNEL_MEDIA_FLAG_KEY,
            &crate::services::feature_flag_service::FlagTarget::User(OWNER.into()),
            enabled,
            OWNER,
        )
        .await
        .unwrap();
        assert_eq!(
            put_binding(
                State(state.clone()),
                Path("bnd_chats".into()),
                bearer(&agent_key),
                Json(binding_body(&agent_key, OWNER))
            )
            .await
            .status(),
            StatusCode::OK
        );
        ensure_partition(&state, &agent_key, PARTITION).await;
        let telegram = MockServer::start().await;
        crate::services::telegram_input::TEST_API_BASES
            .lock()
            .unwrap()
            .insert(row.channel_bot_id.clone(), telegram.uri());
        let _origin = TelegramOrigin(row.channel_bot_id.clone());
        let id = Uuid::new_v4().to_string();
        // The gateway sends only its plain text; files come from the exact
        // authenticated relay record, including this unavailable quoted file.
        state.db.collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
            .insert_one(doc!{"_id":&id,"channel_bot_id":&row.channel_bot_id,"agent_api_key_id":&row.route_api_key_id,
                "direction":"inbound","sender_platform_id":"7","original_message":false,
                "attachments":[{"content_type":"file","provider_ref":"too-large","filename":"large.txt","size_bytes":64_i64*1024*1024}]}).await.unwrap();
        let mut body = event("Keep the forwarded text despite the file failure", "7", &id);
        body["event_context"]["activity"]
            .as_object_mut()
            .unwrap()
            .remove("telegram_input");
        body["event_context"]["activity"]["source"]["route_id"] = json!(row.route_id);
        body["event_context"]["activity"]["conversation"]["id"] = "7".into();
        let response = Box::pin(respond(&state, &agent_key, &body, &id)).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(body_text(response).await.contains("Here is your answer"));
        let captured = calls.lock().await;
        assert_eq!(captured.len(), 1);
        let input = captured[0].to_string();
        assert!(input.contains("Keep the forwarded text despite the file failure"));
        assert!(
            input.contains(if enabled {
                "too large: 1"
            } else {
                "attachments are not shared"
            }),
            "{input}"
        );
        assert!(telegram.received_requests().await.unwrap().is_empty());
        drop(captured);
        model.abort();
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn quoted_files_stay_out_of_gateway_payload_but_reach_the_owner_turn() {
    let (state, calls, model) = setup("nyxbot_telegram_quoted_relay").await;
    crate::services::feature_flag_service::set_platform_override(
        &state.db,
        crate::services::feature_flag_service::ASSISTANT_CHANNEL_MEDIA_FLAG_KEY,
        &crate::services::feature_flag_service::FlagTarget::User(OWNER.into()),
        true,
        OWNER,
    )
    .await
    .unwrap();
    let (row, agent_key) = channel(&state, "gateway").await;
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"owner_sender_ids":["7"]}},
        )
        .await
        .unwrap();
    let token = state
        .encryption_keys
        .encrypt(b"123:quoted-relay-test-secret")
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.channel_bot_id},
            doc! {"$set":{"bot_token_encrypted":bson::Binary {
                subtype:bson::spec::BinarySubtype::Generic, bytes:token
            }}},
        )
        .await
        .unwrap();
    assert_eq!(
        put_binding(
            State(state.clone()),
            Path("bnd_chats".into()),
            bearer(&agent_key),
            Json(binding_body(&agent_key, OWNER))
        )
        .await
        .status(),
        StatusCode::OK
    );
    ensure_partition(&state, &agent_key, PARTITION).await;

    let telegram = MockServer::start().await;
    crate::services::telegram_input::TEST_API_BASES
        .lock()
        .unwrap()
        .insert(row.channel_bot_id.clone(), telegram.uri());
    let _origin = TelegramOrigin(row.channel_bot_id.clone());
    for (id, bytes, count) in [
        ("own-file", "Complete sending message file.", 1),
        (
            "quoted-file",
            "Complete quoted file.\nLast line survives.",
            2,
        ),
    ] {
        Mock::given(method("GET"))
            .and(path("/bot123:quoted-relay-test-secret/getFile"))
            .and(query_param("file_id", id))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,
                "result":{"file_path":format!("documents/{id}.txt"),"file_size":bytes.len()}})))
            .expect(count)
            .mount(&telegram)
            .await;
        Mock::given(method("GET"))
            .and(path(format!(
                "/file/bot123:quoted-relay-test-secret/documents/{id}.txt"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.as_bytes()))
            .expect(count)
            .mount(&telegram)
            .await;
    }
    let adapter = crate::services::channel_adapters::telegram::TelegramAdapter::new();
    let conversation = state
        .db
        .collection::<crate::models::channel_conversation::ChannelConversation>(
            crate::models::channel_conversation::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&row.route_id})
        .await
        .unwrap()
        .unwrap();
    for (message_id, own) in [(101, false), (102, true)] {
        let id = Uuid::new_v4().to_string();
        let question = format!("Summarize the quoted file {message_id}");
        let mut update = json!({"update_id":message_id,"message":{
            "message_id":message_id,"chat":{"id":7,"type":"private"},
            "from":{"id":7,"is_bot":false},"text":question,
            "external_reply":{"origin":{"type":"hidden_user","sender_user_name":"Restaurant"},
                "document":{"file_id":"quoted-file","file_name":"quoted.txt","mime_type":"text/plain"}}
        }});
        if own {
            update["message"].as_object_mut().unwrap().remove("text");
            update["message"]["caption"] = question.clone().into();
            update["message"]["document"] =
                json!({"file_id":"own-file","file_name":"own.txt","mime_type":"text/plain"});
        }
        let inbound = adapter
            .parse_inbound(&serde_json::to_vec(&update).unwrap())
            .await
            .unwrap()
            .remove(0);
        let source = relay::store_inbound_message_with_id(
            &state.db,
            &row.channel_bot_id,
            &conversation.id,
            OWNER,
            "telegram",
            &inbound,
            &row.route_api_key_id,
            &id,
        )
        .await
        .unwrap();
        let payload = relay::build_callback_payload(
            &source,
            &conversation,
            &row.route_api_key_id,
            "gateway",
            &inbound,
            None,
            "https://nyxid.example",
        );
        let wire = serde_json::to_value(&payload).unwrap();
        assert_eq!(wire["content"]["text"], question);
        assert_eq!(wire["content"]["type"], if own { "file" } else { "text" });
        assert_eq!(wire["raw_platform_data"], update);
        assert_eq!(payload.content.attachments.len(), usize::from(own));
        if !own {
            // CMA's media-off and relay extractors both admit this text-only
            // envelope: no top-level media and no unmatched envelope files.
            assert_eq!(wire["content"], json!({"type":"text","text":question}));
        }
        assert_eq!(source.attachments.len(), usize::from(own) + 1);
        for (index, file) in payload.content.attachments.iter().enumerate() {
            assert_eq!(file.url, "own-file");
            assert_eq!(source.attachments[index].provider_ref, file.url);
            assert_eq!(
                file.download_url,
                format!(
                    "https://nyxid.example/api/v1/channel-relay/messages/{id}/attachments/{index}"
                )
            );
        }
        assert_eq!(
            source.attachments.last().unwrap().provider_ref,
            "quoted-file"
        );
        let (original, files) = crate::services::telegram_input::relay_input(
            &state.db,
            &row.channel_bot_id,
            &row.route_api_key_id,
            &id,
            "7",
        )
        .await
        .unwrap();
        assert!(!original);
        assert_eq!(files.len(), usize::from(own) + 1);
        assert_eq!(files.last().unwrap().url, "quoted-file");

        // Current CMA does not produce telegram_input. Exercise the real
        // provider handler using only the authenticated NyxID relay record.
        let mut body = event(&question, "7", &id);
        let activity = &mut body["event_context"]["activity"];
        activity.as_object_mut().unwrap().remove("telegram_input");
        activity["source"]["route_id"] = json!(row.route_id);
        activity["conversation"]["id"] = "7".into();
        let response = Box::pin(respond(&state, &agent_key, &body, &id)).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(body_text(response).await.contains("Here is your answer"));
        let captured = calls.lock().await;
        assert_eq!(captured.len(), if own { 2 } else { 1 });
        let model_input = captured.last().unwrap().to_string();
        assert!(model_input.contains("quoted.txt"));
        assert!(model_input.contains("Destination: Telegram."));
        assert!(model_input.contains("4,096 characters"));
        drop(captured);
        let uploads: Vec<_> = state
            .db
            .collection::<crate::models::assistant_upload::AssistantUpload>(
                crate::models::assistant_upload::COLLECTION_NAME,
            )
            .find(doc! {"user_id":OWNER,"label":"quoted.txt"})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(uploads.len(), if own { 2 } else { 1 });
        let latest = uploads
            .iter()
            .max_by_key(|upload| upload.created_at)
            .unwrap();
        assert!(model_input.contains(&latest.id));
        let messages = engine::messages(&state.db, OWNER, &latest.conversation_id, 50, None)
            .await
            .unwrap();
        let message = messages
            .iter()
            .find(|m| Some(&m.id) == latest.message_id.as_ref())
            .unwrap();
        assert_eq!(message.attachments.len(), usize::from(own) + 1);
        assert!(message.attachments.iter().any(|a| a.id == latest.id));
        for upload in uploads {
            let (_, bytes) = crate::services::assistant_upload_service::owner_read(
                &state.db,
                &state.encryption_keys,
                OWNER,
                &upload.conversation_id,
                &upload.id,
            )
            .await
            .unwrap();
            assert_eq!(bytes, b"Complete quoted file.\nLast line survives.");
        }
    }
    model.abort();
    state.db.drop().await.unwrap();
}
