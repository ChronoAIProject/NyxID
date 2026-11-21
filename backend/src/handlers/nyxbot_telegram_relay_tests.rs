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
        let expected_notes: &[&str] = if enabled {
            &["too large: 1"]
        } else {
            &[
                "1 attachment is not shared",
                "not enabled for this owner",
                "Ask a NyxID platform administrator to review the assistant:channel-media rollout for this owner.",
            ]
        };
        for &expected_note in expected_notes {
            assert!(input.contains(expected_note), "{input}");
        }
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

#[tokio::test]
async fn telegram_exact_group_route_with_gateway_key() {
    Box::pin(exact_group_route(true, false)).await;
}

#[tokio::test]
async fn telegram_exact_group_route_with_another_key() {
    Box::pin(exact_group_route(false, false)).await;
}

#[tokio::test]
async fn telegram_exact_group_route_with_org_gateway_key() {
    Box::pin(exact_group_route(true, true)).await;
}

#[tokio::test]
async fn telegram_exact_group_route_with_another_org_key() {
    Box::pin(exact_group_route(false, true)).await;
}

async fn exact_group_route(same_key: bool, org: bool) {
    use crate::models::channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage};
    let (state, calls, model) = setup("telegram_exact_group_route").await;
    let row = if org {
        let (row, _, _, _) = super::org_channel_tests::org_channel(&state, false).await;
        for (collection, id) in [
            (
                crate::models::channel_bot::COLLECTION_NAME,
                row.channel_bot_id.as_str(),
            ),
            (
                crate::models::channel_conversation::COLLECTION_NAME,
                row.route_id.as_deref().unwrap(),
            ),
        ] {
            state
                .db
                .collection::<bson::Document>(collection)
                .update_one(doc! {"_id":id}, doc! {"$set":{"platform":"telegram-new"}})
                .await
                .unwrap();
        }
        row
    } else {
        channel(&state, "gateway").await.0
    };
    let gateway = MockServer::start().await;
    let other = MockServer::start().await;
    let _other_origin = crate::services::resource_adapter::register_test_origin(&other.uri());
    let routes = std::slice::from_ref(row.route_id.as_ref().unwrap());
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
        .await
        .unwrap();
    let source =
        gateway_policy(&state, &row, &bot, routes, GATEWAY_GROUPS_DEFAULT)["sources"][0].clone();
    assert_eq!(source["route_ids"], json!(routes));
    assert_eq!(source["key_id"], row.route_api_key_id);
    let gateway_route = row.route_id.clone().unwrap();
    // Exercise the published route restriction at the external HTTP boundary.
    Mock::given(method("POST"))
        .respond_with(move |request: &wiremock::Request| {
            let payload: Value = request.body_json().unwrap();
            if payload["conversation"]["id"] == gateway_route {
                ResponseTemplate::new(202).set_body_json(json!({"status":"accepted"}))
            } else {
                ResponseTemplate::new(403)
            }
        })
        .mount(&gateway)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&other)
        .await;
    let other_key = key(&state, "group receiver").await;
    let keys = state
        .db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME);
    for (id, url) in [
        (&row.route_api_key_id, gateway.uri()),
        (&other_key.id, other.uri()),
    ] {
        keys.update_one(doc! {"_id":id}, doc! {"$set":{"callback_url":url}})
            .await
            .unwrap();
    }
    if org {
        keys.update_one(
            doc! {"_id":&other_key.id},
            doc! {"$set":{"user_id":&bot.user_id}},
        )
        .await
        .unwrap();
    }
    let selected_key = if same_key {
        &row.route_api_key_id
    } else {
        &other_key.id
    };
    let exact = channel_routing_service::create_conversation(
        &state.db,
        &bot.user_id,
        Some(&bot.id),
        &bot.platform,
        "-100200",
        "group",
        None,
        selected_key,
        false,
        false,
    )
    .await
    .unwrap();
    let bots = state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME);
    bots.update_one(
        doc! {"_id":&bot.id},
        doc! {"$set":{"webhook_secret_hash":sha256_hex("fixture")}},
    )
    .await
    .unwrap();
    let update = json!({"update_id":42,"message":{"message_id":9,"chat":{"id":-100200,"type":"supergroup"},
        "from":{"id":7,"is_bot":false},"text":"@helper_bot hello","entities":[{"type":"mention","offset":0,"length":11}]}});
    telegram_webhook(&state, &bot, &update, &gateway).await;
    let gw = gateway.received_requests().await.unwrap();
    let external = other.received_requests().await.unwrap();
    let delivered = if same_key {
        assert!(external.is_empty());
        gw
    } else {
        assert!(gw.is_empty());
        external
    };
    assert_eq!(delivered.len(), 1);
    let payload: Value = delivered[0].body_json().unwrap();
    let claims = crate::crypto::jwt::validate_relay_callback_token(
        &state.jwt_keys,
        &state.config,
        delivered[0].headers["x-nyxid-callback-token"]
            .to_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(claims.api_key_id, selected_key.as_str());
    assert_eq!(claims.body_sha256, sha256_hex(&delivered[0].body));
    assert_eq!(payload["conversation"]["id"], exact.id);
    assert_eq!(payload["agent"]["api_key_id"], selected_key.as_str());
    assert_eq!(payload["raw_platform_data"], update);
    let msg = state
        .db
        .collection::<ChannelMessage>(MESSAGES)
        .find_one(doc! {"_id":payload["message_id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(msg.conversation_id, exact.id);
    assert_eq!(
        msg.callback_status.as_deref(),
        Some(if same_key { "failed" } else { "delivered" })
    );
    assert_eq!(msg.callback_http_status, same_key.then_some(403));
    assert_eq!(msg.agent_api_key_id.as_deref(), Some(selected_key.as_str()));
    assert_eq!(
        msg.callback_disposition,
        (!same_key).then_some(crate::models::channel_message::CallbackDisposition::Unknown)
    );
    assert!(msg.nyxbot_admission.is_none());
    assert_eq!(
        state
            .db
            .collection::<ChannelMessage>(MESSAGES)
            .count_documents(doc! {"channel_bot_id":&bot.id, "direction":"outbound"})
            .await
            .unwrap(),
        0
    );
    assert!(channel_conversations(&state, &row.id).await.is_empty());
    assert_eq!(
        state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .count_documents(doc! {"channel_id":&row.id})
            .await
            .unwrap(),
        0
    );
    assert!(calls.lock().await.is_empty());
    let bot = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert_eq!(
        bot.diagnostics.last_route_result.as_deref(),
        Some("matched")
    );
    let code = if same_key {
        "telegram_group_route_outside_source"
    } else {
        "telegram_group_other_agent"
    };
    assert_eq!(
        bot.diagnostics
            .last_inbound_outcome
            .as_ref()
            .map(|o| o.code.as_str()),
        Some(code)
    );
    assert_eq!(bot.diagnostics.inbound_outcomes[code].count, 1);
    let health = crate::handlers::channel_bots::health_for(&bot, true, true);
    assert_eq!(health.recommended_action, "verify");
    assert!(
        health
            .recommended_action_text
            .contains("Conversation Routes")
    );
    assert!(health.recommended_action_text.contains(&exact.id));
    model.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_group_provider_admission_is_visible_on_health() {
    let (state, calls, model) = setup("telegram_group_admission_health").await;
    let (row, credential) = channel(&state, "gateway").await;
    let channels = state.db.collection::<NyxbotChannel>(CHANNELS);
    channels
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"gateway_groups":"all","owner_sender_ids":["7"]}},
        )
        .await
        .unwrap();
    assert_eq!(
        put_binding(
            State(state.clone()),
            Path("bnd_chats".into()),
            bearer(&credential),
            Json(binding_body(&credential, OWNER))
        )
        .await
        .status(),
        StatusCode::OK
    );
    ensure_partition(&state, &credential, PARTITION).await;
    let msgs = state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME);
    let source = Uuid::new_v4().to_string();
    let missing_partition = "conv_ffffffffffffffffffffffffffffffff";
    let mut early = group_event("Hello", "7", "Member", &source, missing_partition, true);
    early["event_context"]["activity"]["source"]["route_id"] = json!(row.route_id);
    msgs.insert_one(doc! {"_id":&source,"direction":"inbound","channel_bot_id":&row.channel_bot_id,
        "user_id":OWNER,"platform":"telegram","conversation_id":&row.route_id,"agent_api_key_id":&row.route_api_key_id,
        "platform_conversation_id":"-100200","sender_platform_id":"7"}).await.unwrap();
    let refused = Box::pin(respond(&state, &credential, &early, &source)).await;
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
        .await
        .unwrap();
    assert_eq!(
        bot.diagnostics
            .last_inbound_outcome
            .as_ref()
            .map(|o| o.code.as_str()),
        Some("telegram_group_partition_missing")
    );
    assert!(
        crate::handlers::channel_bots::health_for(&bot, true, true)
            .recommended_action_text
            .contains("gateway conversation")
    );
    let threads = state.db.collection::<bson::Document>(THREADS);
    for (id, mode) in [("-100201", "all"), ("-100200", "mention")] {
        let chat = fixture_group_chat(&state, &row, id).await;
        threads
            .update_one(doc! {"_id":chat.id}, doc! {"$set":{"reply_mode":mode}})
            .await
            .unwrap();
    }
    msgs.insert_one(doc! {"_id":"outbound","direction":"outbound","platform":"telegram",
        "platform_conversation_id":"-100200","channel_bot_id":&row.channel_bot_id,"platform_message_id":"87"}).await.unwrap();
    for (address, sender, members, mode, disposition, expected) in [
        ("none", "7", None, "mention", "not_addressed", None),
        ("mention", "7", None, "mention", "admitted", None),
        (
            "mention",
            "9",
            Some("owner"),
            "mention",
            "refused_members",
            Some("telegram_group_members_refused"),
        ),
        ("none", "9", Some("owner"), "all", "refused_members", None),
        ("none", "7", None, "all", "admitted", None),
        (
            "mention",
            "9",
            None,
            "mention",
            "refused_owner_absent",
            Some("telegram_group_owner_absent"),
        ),
        // Legacy gateways can imply addressing without evidence of a mention or a reply.
        (
            "legacy",
            "9",
            Some("owner"),
            "mention",
            "refused_members",
            None,
        ),
        (
            "reply",
            "9",
            Some("owner"),
            "mention",
            "refused_members",
            Some("telegram_group_members_refused"),
        ),
    ] {
        if address == "legacy" {
            threads
                .update_one(
                    doc! {"channel_id":&row.id,"partition":chats::group_partition("-100201",None)},
                    doc! {"$set":{"reply_mode":"mention"}},
                )
                .await
                .unwrap();
            channels
                .update_one(
                    doc! {"_id":&row.id},
                    doc! {"$set":{"gateway_groups":GATEWAY_GROUPS_DEFAULT}},
                )
                .await
                .unwrap();
        }
        threads.update_one(doc! {"channel_id":&row.id,"partition":chats::group_partition("-100200",None)},
            doc! {"$set":{"members":members,"reply_mode":mode,"owner_seen":false,"guest_hint_at":bson::DateTime::now()}}).await.unwrap();
        let id = Uuid::new_v4().to_string();
        msgs.insert_one(doc! {"_id":&id,"channel_bot_id":&row.channel_bot_id,"conversation_id":&row.route_id,
            "user_id":OWNER,"platform":"telegram","platform_conversation_id":"-100200","sender_platform_id":sender,
            "agent_api_key_id":&row.route_api_key_id,"direction":"inbound","original_message":true,
            "reply_to_platform_message_id":(address == "reply").then_some("87"),
            "content_type":"text","created_at":bson::DateTime::now()}).await.unwrap();
        let mut body = group_event(
            "Hello",
            sender,
            "Member",
            &id,
            PARTITION,
            address == "mention",
        );
        body["event_context"]["activity"]["source"]["route_id"] = json!(row.route_id);
        body["event_context"]["activity"]["conversation"]["title"] = json!("Fixture group");
        let before = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
            .await
            .unwrap();
        let response = body_text(Box::pin(respond(&state, &credential, &body, &id)).await).await;
        let after = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
            .await
            .unwrap();
        let message = msgs.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
        let admission = message.get_document("nyxbot_admission").unwrap();
        let turn = disposition == "admitted";
        assert_eq!(admission.get_str("disposition").unwrap(), disposition);
        assert_eq!(admission.get_str("turn_id").is_ok(), turn);
        assert_eq!(response.contains("Here is your answer"), turn);
        if let Some(code) = expected {
            assert_eq!(
                after
                    .diagnostics
                    .last_inbound_outcome
                    .as_ref()
                    .map(|o| o.code.as_str()),
                Some(code)
            );
            assert_eq!(
                after.diagnostics.inbound_outcomes[code].count,
                1 + before
                    .diagnostics
                    .inbound_outcomes
                    .get(code)
                    .map_or(0, |c| c.count)
            );
            assert_ne!(
                crate::handlers::channel_bots::health_for(&after, true, true).recommended_action,
                "gateway"
            );
        } else {
            assert_eq!(
                bson::to_bson(&before.diagnostics.last_inbound_outcome).unwrap(),
                bson::to_bson(&after.diagnostics.last_inbound_outcome).unwrap()
            );
            assert_eq!(
                bson::to_document(&before.diagnostics.inbound_outcomes).unwrap(),
                bson::to_document(&after.diagnostics.inbound_outcomes).unwrap()
            );
        }
    }
    assert_eq!(calls.lock().await.len(), 2);
    model.abort();
    state.db.drop().await.unwrap();
}

async fn fixture_group_chat(state: &AppState, row: &NyxbotChannel, chat_id: &str) -> NyxbotThread {
    chats::record_chat(
        state,
        row,
        &chats::group_partition(chat_id, None),
        &chats::ChatFacts {
            kind: "group",
            chat_id: chat_id.into(),
            thread_id: None,
            owner: false,
            title: Some("Fixture group".into()),
        },
        None,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn telegram_group_matched_storage_refusal_is_visible_without_a_message_row() {
    let (state, calls, model) = setup("telegram_group_storage").await;
    let (row, _) = channel(&state, "gateway").await;
    let db = &state.db;
    let receiver = MockServer::start().await;
    db.collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.route_api_key_id},
            doc! {"$set":{"callback_url":receiver.uri()}},
        )
        .await
        .unwrap();
    db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.channel_bot_id},
            doc! {"$set":{"webhook_secret_hash":sha256_hex("fixture")}},
        )
        .await
        .unwrap();
    let messages = crate::models::channel_message::COLLECTION_NAME;
    if !db
        .list_collection_names()
        .await
        .unwrap()
        .iter()
        .any(|name| name == messages)
    {
        db.create_collection(messages).await.unwrap();
    }
    db.run_command(doc! {"collMod":messages, "validator":{"unavailable":{"$exists":true}}})
        .await
        .unwrap();
    let update = json!({"update_id":42,"message":{"message_id":9,"chat":{"id":-100200,"type":"group"},
        "from":{"id":7,"is_bot":false},"text":"hello","reply_to_message":{"message_id":8,"from":{"id":123,"is_bot":true}}}});
    let bot = channel_bot_service::get_bot(db, &row.channel_bot_id)
        .await
        .unwrap();
    telegram_webhook(&state, &bot, &update, &receiver).await;
    assert_eq!(
        db.collection::<bson::Document>(messages)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert!(receiver.received_requests().await.unwrap().is_empty());
    assert!(calls.lock().await.is_empty());
    let bot = channel_bot_service::get_bot(db, &row.channel_bot_id)
        .await
        .unwrap();
    assert_eq!(
        bot.diagnostics.last_route_result.as_deref(),
        Some("matched")
    );
    assert_eq!(
        bot.diagnostics
            .last_inbound_outcome
            .as_ref()
            .map(|o| o.code.as_str()),
        Some("telegram_group_storage_unavailable")
    );
    assert_eq!(
        crate::handlers::channel_bots::health_for(&bot, true, true).recommended_action,
        "verify"
    );
    model.abort();
    db.drop().await.unwrap();
}

async fn telegram_webhook(
    state: &AppState,
    bot: &ChannelBot,
    update: &Value,
    receiver: &MockServer,
) {
    let _receiver_origin = crate::services::resource_adapter::register_test_origin(&receiver.uri());
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-telegram-bot-api-secret-token",
        "fixture".parse().unwrap(),
    );
    let response = Box::pin(crate::handlers::channel_webhooks::channel_webhook_with_api(
        State(state.clone()),
        Path((bot.platform.clone(), bot.id.clone())),
        format!("/api/v1/webhooks/channel/{}/{}", bot.platform, bot.id)
            .parse()
            .unwrap(),
        headers,
        Bytes::from(serde_json::to_vec(update).unwrap()),
        &receiver.uri(),
    ))
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn telegram_group_callback_failures_and_acceptance_are_distinct() {
    let (state, _, server) = setup("telegram_group_callback").await;
    let (row, _) = channel(&state, "gateway").await;
    let receiver = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|req: &wiremock::Request| {
            match req.body_json::<Value>().unwrap()["raw_platform_data"]["update_id"]
                .as_i64()
                .unwrap()
            {
                1 => ResponseTemplate::new(502),
                2 => ResponseTemplate::new(202)
                    .set_body_json(json!({"status":"refused","reason":"group_not_addressed"})),
                _ => ResponseTemplate::new(202).set_body_json(json!({"status":"accepted"})),
            }
        })
        .mount(&receiver)
        .await;
    let db = &state.db;
    db.collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.route_api_key_id},
            doc! {"$set":{"callback_url":receiver.uri()}},
        )
        .await
        .unwrap();
    db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.channel_bot_id},
            doc! {"$set":{"webhook_secret_hash":sha256_hex("fixture")}},
        )
        .await
        .unwrap();
    for (id, outcome) in [
        (1, Some("telegram_group_callback_failed")),
        (2, Some("telegram_group_callback_refused")),
        (3, None),
    ] {
        let bot = channel_bot_service::get_bot(db, &row.channel_bot_id)
            .await
            .unwrap();
        let update = json!({"update_id":id,"message":{"message_id":id,"chat":{"id":-100200,"type":"group"},
            "from":{"id":7,"is_bot":false},"text":"hello"}});
        telegram_webhook(&state, &bot, &update, &receiver).await;
        let after = channel_bot_service::get_bot(db, &row.channel_bot_id)
            .await
            .unwrap();
        let health = crate::handlers::channel_bots::health_for(&after, true, true);
        if let Some(code) = outcome {
            assert_eq!(
                after
                    .diagnostics
                    .last_inbound_outcome
                    .as_ref()
                    .unwrap()
                    .code,
                code
            );
            assert_eq!(after.diagnostics.inbound_outcomes[code].count, 1);
            assert_eq!(health.recommended_action, "verify");
        } else {
            assert_eq!(health.recommended_action, "gateway");
            assert_eq!(
                bson::to_document(&after.diagnostics.inbound_outcomes).unwrap(),
                bson::to_document(&bot.diagnostics.inbound_outcomes).unwrap()
            );
        }
    }
    assert_eq!(receiver.received_requests().await.unwrap().len(), 3);
    server.abort();
    db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_group_redelivery_preserves_native_event_and_bot_diagnostics() {
    let (state, calls, model) = setup("telegram_group_redelivery").await;
    let (row, credential) = channel(&state, "gateway").await;
    let db = &state.db;
    db.collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"gateway_bot_mentions.enabled":true,"owner_sender_ids":["7"]}},
        )
        .await
        .unwrap();
    fixture_group_chat(&state, &row, "-100200").await;
    assert_eq!(
        put_binding(
            State(state.clone()),
            Path("bnd_chats".into()),
            bearer(&credential),
            Json(binding_body(&credential, OWNER))
        )
        .await
        .status(),
        StatusCode::OK
    );
    ensure_partition(&state, &credential, PARTITION).await;
    let receiver = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(202).set_body_json(json!({"status":"accepted"})))
        .mount(&receiver)
        .await;
    db.collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.route_api_key_id},
            doc! {"$set":{"callback_url":receiver.uri()}},
        )
        .await
        .unwrap();
    db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.channel_bot_id},
            doc! {"$set":{"webhook_secret_hash":sha256_hex("fixture")}},
        )
        .await
        .unwrap();
    let bot = channel_bot_service::get_bot(db, &row.channel_bot_id)
        .await
        .unwrap();
    let update = json!({"update_id":42,"message":{"message_id":9,"chat":{"id":-100200,"type":"supergroup"},
        "from":{"id":7,"is_bot":false},"text":"@helper_bot hello","entities":[{"type":"mention","offset":0,"length":11}]}});
    let messages = db.collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME);
    for attempt in 0..2 {
        telegram_webhook(&state, &bot, &update, &receiver).await;
        let requests = receiver.received_requests().await.unwrap();
        assert_eq!(requests.len(), attempt + 1);
        let payload: Value = requests[attempt].body_json().unwrap();
        let id = payload["message_id"].as_str().unwrap();
        let mut body = group_event("@helper_bot hello", "7", "Owner", id, PARTITION, true);
        body["event_context"]["activity"]["source"]["route_id"] = json!(row.route_id);
        let response = Box::pin(respond(&state, &credential, &body, id)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let text = body_text(response).await;
        if attempt == 0 {
            assert!(text.contains("Here is your answer"));
        }
        let msg = messages.find_one(doc! {"_id":id}).await.unwrap().unwrap();
        assert_eq!(msg.get_str("platform_message_id").unwrap(), "9");
        assert_eq!(msg.get_str("callback_status").unwrap(), "delivered");
        assert_eq!(msg.get_document("nyxbot_admission").is_ok(), attempt == 0);
    }
    assert_eq!(
        messages
            .count_documents(
                doc! {"direction":"inbound","channel_bot_id":&bot.id,"platform_message_id":"9"}
            )
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        db.collection::<NyxbotEvent>(EVENTS)
            .count_documents(doc! {"channel_id":&row.id})
            .await
            .unwrap(),
        1
    );
    assert_eq!(calls.lock().await.len(), 1);
    let after = channel_bot_service::get_bot(db, &bot.id).await.unwrap();
    assert!(after.diagnostics.inbound_outcomes.is_empty());
    assert!(after.diagnostics.last_inbound_outcome.is_none());

    // The non-deduplicating store refuses only when its live owner fence is lost.
    db.collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id":OWNER},
            doc! {"$set":{"platform_personal_deleting":true}},
        )
        .await
        .unwrap();
    telegram_webhook(&state, &bot, &update, &receiver).await;
    let after = channel_bot_service::get_bot(db, &bot.id).await.unwrap();
    assert_eq!(
        after
            .diagnostics
            .last_inbound_outcome
            .as_ref()
            .unwrap()
            .code,
        "telegram_group_owner_unavailable"
    );
    assert_eq!(
        after.diagnostics.inbound_outcomes["telegram_group_owner_unavailable"].count,
        1
    );
    assert!(
        crate::handlers::channel_bots::health_for(&after, true, true)
            .recommended_action_text
            .contains("owner account")
    );
    assert_eq!(receiver.received_requests().await.unwrap().len(), 2);
    model.abort();
    db.drop().await.unwrap();
}
