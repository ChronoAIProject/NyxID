//! Relay v2: real Mongo, real adapter downloads, mocked platforms and model.
use super::*;
use crate::services::{
    assistant_upload_service as uploads, channel_adapters as adapters,
    channel_media_events as media_events, channel_platform::*,
};

// WhatsApp's production adapter is a unit struct. Only substitute the Graph
// origin; use its production metadata lookup/download and credential handling.
struct WhatsAppGraph(String);
#[async_trait::async_trait]
impl PlatformAdapter for WhatsAppGraph {
    fn outbound_capabilities(&self) -> OutboundCapabilities {
        adapters::whatsapp::WhatsAppAdapter.outbound_capabilities()
    }
    fn media_capabilities(&self) -> MediaCapabilities {
        adapters::whatsapp::WhatsAppAdapter.media_capabilities()
    }
    fn platform_id(&self) -> &str {
        "whatsapp"
    }
    async fn fetch_attachment(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        item: &InboundAttachment,
        limit: u64,
    ) -> AppResult<FetchedMedia> {
        adapters::whatsapp::fetch_media_at(http, credentials, item, limit, &self.0).await
    }
    async fn verify_webhook(
        &self,
        _: &crate::models::channel_bot::ChannelBot,
        _: Option<&PlatformVerifySecrets>,
        _: &HeaderMap,
        _: &[u8],
    ) -> AppResult<()> {
        unreachable!()
    }
    async fn parse_inbound(&self, _: &[u8]) -> AppResult<Vec<InboundMessage>> {
        unreachable!()
    }
    async fn send_reply(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        chat: &str,
        reply: &OutboundReply,
    ) -> AppResult<Option<String>> {
        self.send_reply_outcome(http, credentials, chat, reply)
            .await?
            .into_result()
    }
    async fn send_reply_outcome(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        chat: &str,
        reply: &OutboundReply,
    ) -> AppResult<SendOutcome> {
        adapters::whatsapp::send_outcome_at(http, credentials, chat, reply, &self.0).await
    }
    async fn register_webhook(
        &self,
        _: &reqwest::Client,
        _: &str,
        _: &str,
        _: &str,
    ) -> AppResult<()> {
        unreachable!()
    }
    async fn verify_bot_token(
        &self,
        _: &reqwest::Client,
        _: &BotCredentials<'_>,
    ) -> AppResult<BotIdentity> {
        unreachable!()
    }
}

async fn platform(state: &AppState, row: &mut NyxbotChannel, name: &str) {
    row.platform = name.into();
    if name == "discord" {
        row.owner_sender_ids.push("111111111111111111".into());
        row.gateway_channel_id = Some("cmaeg1.ch.relay-media".into());
    }
    row.private_chats = Some("everyone".into());
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&row.id}, &*row)
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>("channel_bots")
        .update_one(
            doc! {"_id":&row.channel_bot_id},
            doc! {"$set":{"platform":name}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>("channel_conversations")
        .update_one(doc! {"_id":&row.route_id}, doc! {"$set":{"platform":name}})
        .await
        .unwrap();
    if name == "x" {
        let provider = Uuid::new_v4().to_string();
        let connection = Uuid::new_v4().to_string();
        state.db.collection::<bson::Document>("provider_configs").insert_one(doc!{
            "_id":&provider,"slug":"twitter","name":"X","provider_type":"oauth2","is_active":true,
            "credential_mode":"user","supports_pkce":true,"created_by":OWNER,
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
        }).await.unwrap();
        KeySecret::AccessToken
            .insert(
                &state.db,
                &state.encryption_keys,
                doc! {
                "_id":&connection,"user_id":OWNER,"label":"X","credential_type":"oauth2",
                "credential_source":"platform","provider_config_id":provider,"status":"active",
                "token_scopes":adapters::x::REQUIRED_SCOPES.join(" "),
                "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
                },
                b"app:secret",
            )
            .await;
        state
            .db
            .collection::<bson::Document>("channel_bots")
            .update_one(
                doc! {"_id":&row.channel_bot_id},
                doc! {"$set":{"credential_source":"connection","connection_id":connection}},
            )
            .await
            .unwrap();
    }
}

async fn provider(state: &AppState, row: &NyxbotChannel) -> MockServer {
    // Keep outbound reply plumbing separate from the adapter under test.
    let server = ingestion_provider(state, row, native_png()).await;
    for (id, bytes) in [("456", native_png()), ("457", vec![0xff, 0, 0xfe, 1])] {
        let mut get = Mock::given(method("GET")).and(path(format!("/download/{id}")));
        if row.platform != "discord" {
            get = get.and(header("Authorization", "Bearer app:secret"));
        }
        get.respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/{id}")))
            .and(header("Authorization", "Bearer app:secret"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"url":format!("{}/download/{id}", server.uri())})),
            )
            .mount(&server)
            .await;
    }
    if row.platform == "telegram" {
        for (id, bytes) in [("456", native_png()), ("457", vec![0xff, 0, 0xfe, 1])] {
            Mock::given(method("GET"))
                .and(path("/botapp:secret/getFile"))
                .and(query_param("file_id", id))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(
                        json!({"ok":true,"result":{"file_path":format!("media/{id}")}}),
                    ),
                )
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path(format!("/file/botapp:secret/media/{id}")))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
                .mount(&server)
                .await;
        }
    }
    let adapter: Arc<dyn PlatformAdapter> = match row.platform.as_str() {
        "discord" => Arc::new(adapters::discord::DiscordAdapter::media_test_adapter(
            &server.uri(),
        )),
        "slack" => Arc::new(adapters::slack::SlackAdapter::media_test_adapter(
            &server.uri(),
        )),
        "whatsapp" => Arc::new(WhatsAppGraph(server.uri())),
        "telegram" => Arc::new(adapters::telegram::TelegramAdapter::media_test_adapter(
            &server.uri(),
        )),
        "x" => Arc::new(adapters::x::XAdapter {
            api_base: Some(server.uri()),
        }),
        _ => unreachable!(),
    };
    crate::services::assistant_channel_media::TEST_ADAPTERS
        .lock()
        .unwrap()
        .insert(row.channel_bot_id.clone(), adapter);
    server
}

async fn saved_source(
    state: &AppState,
    row: &NyxbotChannel,
    provider: &MockServer,
    id: &str,
    sender: &str,
) -> ChannelMessage {
    let mut source = media_source(state, row, id, id, "").await;
    source.sender_platform_id = Some(
        if row.platform == "discord" {
            if sender == "human" {
                "111111111111111111"
            } else {
                "333333333333333333"
            }
        } else if row.platform == "telegram" {
            "444"
        } else {
            sender
        }
        .into(),
    );
    source.platform_conversation_id = Some(if row.platform == "discord" {
        "222222222222222222".into()
    } else if row.platform == "telegram" {
        "444".into()
    } else {
        format!("oc_{sender}")
    });
    let provider_ref = |id: &str| match row.platform.as_str() {
        "telegram" => id.to_owned(),
        "whatsapp" => format!("{}/{id}", provider.uri()),
        _ => format!("{}/download/{id}", provider.uri()),
    };
    let mut image = source.attachments[0].clone();
    image.provider_ref = provider_ref("456");
    image.image_key = None;
    image.file_key = Some("456".into());
    image.filename = None; // WhatsApp images commonly have no filename.
    image.mime_type = Some("image/png".into());
    image.size_bytes = None;
    let mut original = image.clone();
    original.provider_ref = provider_ref("457");
    original.file_key = Some("457".into());
    original.content_type = "file".into();
    original.filename = Some("original.bin".into());
    original.mime_type = Some("image/png".into()); // MIME claims cannot authorize image input.
    source.attachments = vec![image, original];
    state
        .db
        .collection::<ChannelMessage>(MESSAGES)
        .replace_one(doc! {"_id":id}, &source)
        .await
        .unwrap();
    source
}

async fn callback(state: &AppState, row: &NyxbotChannel, source: &ChannelMessage) -> StatusCode {
    callback_response(state, row, source).await.status()
}

async fn callback_response(
    state: &AppState,
    row: &NyxbotChannel,
    source: &ChannelMessage,
) -> Response {
    let body = json!({
        "message_id":source.id,"correlation_id":format!("jti-{}",source.id),"platform":source.platform,
        "agent":{"api_key_id":row.route_api_key_id,"name":"route"},
        "conversation":{"id":row.route_id,"platform_id":source.platform_conversation_id,"type":"private"},
        "sender":{"platform_id":source.sender_platform_id},"content":{"type":"image","text":""},
        // Descriptors/URLs in the callback must never override stored selectors.
        "attachments":[{"url":"https://untrusted.invalid/not-the-source","download_url":"https://untrusted.invalid/image.png"}],
        "raw_platform_data":if row.platform == "telegram" {json!({"message":{"message_id":1,"from":{"id":444,"is_bot":false},"chat":{"id":444,"type":"private"}}})} else {json!({})},"timestamp":"2026-11-06T00:00:00Z"
    });
    let bytes = serde_json::to_vec(&body).unwrap();
    let token = crate::crypto::jwt::generate_relay_callback_token(
        &state.jwt_keys,
        &state.config,
        &format!("jti-{}", source.id),
        &row.route_api_key_id,
        &source.id,
        &source.platform,
        &sha256_hex(&bytes),
    )
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-nyxid-callback-token", token.parse().unwrap());
    relay_callback(
        State(state.clone()),
        Path(row.id.clone()),
        headers,
        Bytes::from(bytes),
    )
    .await
}

#[tokio::test]
async fn direct_callback_requires_the_same_stored_inbound_on_redelivery() {
    let (state, row, _, _, server) = media_fixture("callback_source_binding", "direct").await;
    let source = media_source(&state, &row, "source", "native", "").await;
    let (_guard, done) = watch_relay_callback(&row.id, &source.id);
    assert_eq!(callback(&state, &row, &source).await, StatusCode::ACCEPTED);
    done.await.unwrap();

    let records = state.db.collection::<bson::Document>(MESSAGES);
    let original = bson::to_document(&source).unwrap();
    let mut results = Vec::new();
    for field in [
        "channel_bot_id",
        "platform",
        "conversation_id",
        "sender_platform_id",
        "agent_api_key_id",
        "user_id",
        "direction",
        "platform_conversation_id",
        "thread_id",
    ] {
        let mut changed = original.clone();
        changed.insert(field, "other");
        records
            .replace_one(doc! {"_id": &source.id}, changed)
            .await
            .unwrap();
        results.push((field, callback(&state, &row, &source).await));
    }
    records.delete_one(doc! {"_id": &source.id}).await.unwrap();
    results.push(("absent", callback(&state, &row, &source).await));
    assert!(
        results
            .iter()
            .all(|(_, status)| *status == StatusCode::FORBIDDEN),
        "{results:?}"
    );
    let mut fresh = source.clone();
    fresh.id = Uuid::new_v4().to_string();
    let refused = callback_response(&state, &row, &fresh).await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    let problem: Value = serde_json::from_str(&body_text(refused).await).unwrap();
    assert_eq!(
        problem,
        json!({"error": {"code": "relay_source_refused"}, "code": "relay_source_refused"})
    );
    assert_eq!(
        state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .count_documents(doc! {"channel_id": &row.id})
            .await
            .unwrap(),
        1
    );
    let audit = state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(
            doc! {"event_type": "nyxbot_provider_refused", "event_data.endpoint": "relay_callback"},
        )
        .await
        .unwrap()
        .expect("authenticated callback refusal");
    let details = audit.get_document("event_data").unwrap();
    assert_eq!(details.get_str("code").unwrap(), "relay_source_refused");
    assert_eq!(details.get_str("channel_agent_id").unwrap(), &row.id);
    server.abort();
}

#[tokio::test]
async fn direct_callback_admits_each_platform_once() {
    let (state, _, server) = setup("callback_platforms").await;
    for name in [
        "telegram",
        "telegram-new",
        "lark",
        "feishu",
        "discord",
        "slack",
        "whatsapp",
        "x",
        "aurinko",
    ] {
        let (mut row, _) = channel(&state, "direct").await;
        platform(&state, &mut row, name).await;
        let source = media_source(&state, &row, name, "native", "").await;
        if name == "telegram-new" {
            row.platform = "telegram".into();
            state
                .db
                .collection::<NyxbotChannel>(CHANNELS)
                .replace_one(doc! {"_id": &row.id}, &row)
                .await
                .unwrap();
        }
        let (_guard, done) = watch_relay_callback(&row.id, &source.id);
        assert_eq!(
            callback(&state, &row, &source).await,
            StatusCode::ACCEPTED,
            "{name}"
        );
        done.await.unwrap();
        assert_eq!(
            callback(&state, &row, &source).await,
            StatusCode::ACCEPTED,
            "{name}"
        );
        assert_eq!(
            state
                .db
                .collection::<NyxbotEvent>(EVENTS)
                .count_documents(doc! {"channel_id": &row.id})
                .await
                .unwrap(),
            1,
            "{name}"
        );
    }
    server.abort();
}

#[tokio::test]
async fn pending_relay_redelivery_completes_once_on_the_current_route() {
    use crate::services::{channel_retry_ingress as retry, channel_routing_service as routes};
    let (state, _, server) = setup("callback_pending_retry").await;
    let (mut row, _) = channel(&state, "direct").await;
    platform(&state, &mut row, "aurinko").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let _delivery_origin =
        crate::services::resource_adapter::register_test_origin(&format!("http://{address}"));
    let app = axum::Router::new()
        .route("/relay/{id}", axum::routing::post(relay_callback))
        .with_state(state.clone());
    let relay = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    state
        .db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &row.route_api_key_id},
            doc! {"$set": {"callback_url": format!("http://{address}/relay/{}", row.id)}},
        )
        .await
        .unwrap();
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
        .await
        .unwrap();
    let adapter = adapters::resolve_adapter("aurinko", &state.token_exchange_cache).unwrap();
    let inbound = InboundMessage {
        platform_message_id: "mail-message".into(),
        conversation_id: "mail-thread".into(),
        conversation_type: "private".into(),
        sender_platform_id: "sender@example.test".into(),
        sender_display_name: None,
        content_type: "text".into(),
        text: None,
        attachments: vec![],
        embedded_attachments: vec![],
        embedded_attachments_omitted: 0,
        reply_to_platform_message_id: None,
        thread_id: Some("mail-thread".into()),
        raw_data: json!({}),
    };
    let context = retry::IngressContext {
        db: &state.db,
        config: &state.config,
        jwt_keys: &state.jwt_keys,
        encryption_keys: &state.encryption_keys,
        rate_limiter: &state.per_channel_event_limiter,
    };
    let id = Uuid::new_v4().to_string();
    let deliver = || {
        Box::pin(retry::deliver(
            &context,
            &bot,
            adapter.as_ref(),
            &inbound,
            &id,
        ))
    };
    let exact = routes::create_conversation(
        &state.db,
        OWNER,
        Some(&bot.id),
        "aurinko",
        &inbound.conversation_id,
        "private",
        None,
        &row.route_api_key_id,
        false,
        false,
    )
    .await
    .unwrap();
    let messages = state.db.collection::<ChannelMessage>(MESSAGES);
    let source = || messages.find_one(doc! {"_id": &id});
    let events = state.db.collection::<NyxbotEvent>(EVENTS);
    let count = || events.count_documents(doc! {"channel_id": &row.id});
    assert!(matches!(
        deliver().await,
        Err(AppError::ChannelRelayFailed(_))
    ));
    let pending = source().await.unwrap().unwrap();
    assert_eq!(pending.conversation_id, exact.id);
    assert_eq!(pending.callback_status.as_deref(), Some("failed"));
    assert_eq!(count().await.unwrap(), 0);

    // Once normal routing selects the channel's default again, the producer
    // updates its retained route and sends the same message ID.
    state
        .db
        .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
        .update_one(doc! {"_id": exact.id}, doc! {"$set": {"is_active": false}})
        .await
        .unwrap();
    let (_watch, done) = watch_relay_callback(&row.id, &id);
    deliver().await.unwrap();
    done.await.unwrap();
    let accepted = source().await.unwrap().unwrap();
    assert_eq!(Some(accepted.conversation_id), row.route_id);
    assert_eq!(accepted.callback_status.as_deref(), Some("delivered"));
    relay.abort();
    relay.await.unwrap_err();
    // A further provider redelivery completes with the callback listener down.
    deliver().await.unwrap();
    assert_eq!(count().await.unwrap(), 1);
    server.abort();
}

fn provider_event(row: &NyxbotChannel, source: &ChannelMessage) -> Value {
    let mut body = cma_media_event(
        "",
        source.sender_platform_id.as_deref().unwrap(),
        &source.id,
    );
    body["event_context"]["activity"]["source"]["platform"] = json!(row.platform);
    body["event_context"]["activity"]["source"]["route_id"] = json!(row.route_id);
    body["event_context"]["activity"]["conversation"]["id"] =
        json!(source.platform_conversation_id);
    if row.platform == "discord" {
        body["event_context"]["channel_id"] = json!(row.gateway_channel_id);
    }
    body
}

async fn deliver(state: &AppState, row: &NyxbotChannel, key: &str, source: &ChannelMessage) {
    if row.transport == "direct" {
        let (_guard, done) = watch_relay_callback(&row.id, &source.id);
        assert_eq!(callback(state, row, source).await, StatusCode::ACCEPTED);
        tokio::time::timeout(Duration::from_secs(30), done)
            .await
            .unwrap()
            .unwrap();
    } else {
        let body = provider_event(row, source);
        let response = respond(state, key, &body, &source.id).await;
        assert_eq!(response.status(), StatusCode::OK);
        let text = body_text(response).await;
        assert!(text.contains("Here is your answer"), "{text}");
    }
}

#[tokio::test]
async fn channel_media_v2_all_adapters_owner_history_model_originals_and_duplicate() {
    for name in ["discord", "slack", "whatsapp", "x"] {
        for transport in ["direct", "gateway"] {
            let (state, mut row, key, calls, server) =
                media_fixture(&format!("relay_v2_{name}_{transport}"), transport).await;
            platform(&state, &mut row, name).await;
            enable_media(&state).await;
            crate::services::security_readiness_service::ready_test_replica(&state.db).await;
            if transport == "gateway" {
                gateway_ready(&state, &key).await;
            }
            let provider = provider(&state, &row).await;
            let source = saved_source(&state, &row, &provider, "owner-media", "human").await;
            deliver(&state, &row, &key, &source).await;
            let event = state
                .db
                .collection::<NyxbotEvent>(EVENTS)
                .find_one(doc! {"event_id":&source.id})
                .await
                .unwrap()
                .unwrap();
            let conversation = event.conversation_id.as_ref().unwrap();
            let messages = engine::messages(&state.db, OWNER, conversation, 100, None)
                .await
                .unwrap();
            let input = messages
                .into_iter()
                .find(|m| !m.attachments.is_empty())
                .unwrap_or_else(|| {
                    panic!(
                        "{name}/{transport}: no attachments; {:?}",
                        calls.try_lock().ok().as_deref()
                    )
                });
            assert_eq!(input.attachments.len(), 2, "{name}/{transport}");
            let image = &input.attachments[0];
            let original = &input.attachments[1];
            assert_eq!(image.content_type, "image/png");
            assert_eq!(original.content_type, "application/octet-stream");
            let receipt = input.media_bound.as_ref().unwrap();
            assert!(receipt.failed.is_empty());
            assert_eq!(receipt.ok[1].status, "stored_original");
            assert!(receipt.context.contains("saved for download"));
            let json = serde_json::to_value(
                crate::handlers::assistant_nyxagent::MessageResponse::from(input.clone()),
            )
            .unwrap();
            assert_eq!(json["attachments"][0]["id"], image.id);
            assert_eq!(json["attachments"][0]["content_type"], "image/png");
            assert!(!json.to_string().contains(&provider.uri()));
            assert_eq!(
                uploads::owner_read(
                    &state.db,
                    &state.encryption_keys,
                    OWNER,
                    conversation,
                    &image.id
                )
                .await
                .unwrap()
                .1,
                native_png()
            );
            assert_eq!(
                uploads::owner_read(
                    &state.db,
                    &state.encryption_keys,
                    OWNER,
                    conversation,
                    &original.id
                )
                .await
                .unwrap()
                .1,
                vec![0xff, 0, 0xfe, 1]
            );
            assert!(
                uploads::owner_read(
                    &state.db,
                    &state.encryption_keys,
                    "foreign",
                    conversation,
                    &image.id
                )
                .await
                .is_err()
            );
            let captured = calls.lock().await.last().unwrap().to_string();
            assert!(captured.contains("input_image"), "{captured}");
            assert!(captured.contains(&image.id));
            assert!(!captured.contains(&format!("attachments/{}", original.id)));
            assert!(!captured.contains(&provider.uri()));
            let downloads = provider.received_requests().await.unwrap().len();
            let count = calls.lock().await.len();
            // Identical callback/provider retries do not fetch or start another model turn.
            if transport == "direct" {
                assert!(callback(&state, &row, &source).await.is_success());
            } else {
                let body = provider_event(&row, &source);
                let response = respond(&state, &key, &body, "retry").await;
                assert_eq!(response.status(), StatusCode::OK);
                let _ = body_text(response).await;
                let alias =
                    crate::services::nyxbot_dispatch_identity::dispatch_key(&row.id, "retry");
                let winner = state
                    .db
                    .collection::<NyxbotEvent>(EVENTS)
                    .find_one(doc! {"dispatch_identity.keys":alias})
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    winner.id, event.id,
                    "{name}/{transport}: retry alias must resolve the admitted media event"
                );
            }
            assert_eq!(calls.lock().await.len(), count);
            assert_eq!(provider.received_requests().await.unwrap().len(), downloads);
            engine::delete(&state.db, OWNER, conversation)
                .await
                .unwrap();
            assert_eq!(
                state
                    .db
                    .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
                    .count_documents(doc! {"conversation_id":conversation})
                    .await
                    .unwrap(),
                0
            );
            server.abort();
        }
    }
}

#[tokio::test]
async fn channel_media_v2_flag_readiness_guests_and_forged_identity_never_download() {
    for name in ["discord", "slack", "whatsapp", "x"] {
        let (state, mut row, _, calls, server) =
            media_fixture(&format!("relay_v2_closed_{name}"), "direct").await;
        platform(&state, &mut row, name).await;
        let provider = provider(&state, &row).await;
        let source = saved_source(&state, &row, &provider, "owner-media", "human").await;
        let mut request = media_request(&row, &source);
        request.sender_id = source.sender_platform_id.clone().unwrap();
        request.chat_id = source.platform_conversation_id.clone().unwrap();
        assert!(
            media_events::admission(&state.db, &request)
                .await
                .unwrap()
                .is_none()
        );
        enable_media(&state).await;
        // A fresh old generation cannot be hidden by a new replica's heartbeat.
        crate::services::security_readiness_service::ready_test_replica(&state.db).await;
        let replicas = state.db.collection::<bson::Document>("security_replicas");
        let mut legacy = replicas.find_one(doc! {}).await.unwrap().unwrap();
        legacy.insert("_id", "old-generation");
        legacy.insert("generation_id", "legacy");
        legacy.insert("capabilities", bson::bson!([]));
        replicas.insert_one(legacy).await.unwrap();
        assert!(
            media_events::admission(&state.db, &request)
                .await
                .unwrap()
                .is_none()
        );
        replicas
            .delete_one(doc! {"_id":"old-generation"})
            .await
            .unwrap();
        assert!(
            media_events::admission(&state.db, &request)
                .await
                .unwrap()
                .is_some()
        );
        let mut native = request.clone();
        native.channel.gateway_source =
            Some(crate::models::nyxbot_channel::GatewaySource::Unsupported);
        assert!(
            media_events::admission(&state.db, &native)
                .await
                .unwrap()
                .is_none()
        );
        for field in ["sender", "chat", "thread", "route", "key", "bot"] {
            let mut forged = request.clone();
            match field {
                "sender" => forged.sender_id = "foreign".into(),
                "chat" => forged.chat_id = "foreign".into(),
                "thread" => forged.native_thread_id = Some("foreign".into()),
                "route" => forged.channel.route_id = Some("foreign".into()),
                "key" => forged.channel.route_api_key_id = "foreign".into(),
                "bot" => forged.channel.channel_bot_id = "foreign".into(),
                _ => unreachable!(),
            }
            assert!(
                media_events::admission(&state.db, &forged)
                    .await
                    .unwrap()
                    .is_none(),
                "{name}/{field}"
            );
        }
        let guest = saved_source(&state, &row, &provider, "guest-media", "guest").await;
        let mut request = media_request(&row, &guest);
        request.sender_id = guest.sender_platform_id.clone().unwrap();
        request.chat_id = guest.platform_conversation_id.clone().unwrap();
        assert!(
            media_events::admission(&state.db, &request)
                .await
                .unwrap()
                .is_none()
        );
        let (_guard, done) = watch_relay_callback(&row.id, &guest.id);
        assert_eq!(callback(&state, &row, &guest).await, StatusCode::ACCEPTED);
        tokio::time::timeout(Duration::from_secs(30), done)
            .await
            .unwrap()
            .unwrap();
        let captured = calls.lock().await.last().unwrap().to_string();
        assert!(captured.contains(crate::services::assistant_channel_media::GUEST_NOTE));
        assert!(!captured.contains("input_image"));
        assert!(
            provider
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method != "GET")
        );
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        server.abort();
    }
}

#[tokio::test]
async fn channel_media_v2_rechecks_after_download_partial_failure_and_retention() {
    for revoke in [false, true] {
        let (state, mut row, key, calls, server) =
            media_fixture("relay_v2_download_recheck", "direct").await;
        platform(&state, &mut row, "discord").await;
        enable_media(&state).await;
        crate::services::security_readiness_service::ready_test_replica(&state.db).await;
        let provider = provider(&state, &row).await;
        let mut source = saved_source(&state, &row, &provider, "owner-media", "human").await;
        let db = state.db.clone();
        let channel = row.id.clone();
        let app = axum::Router::new().route(
            "/image",
            axum::routing::get(move || {
                let db = db.clone();
                let channel = channel.clone();
                async move {
                    if revoke {
                        db.collection::<bson::Document>(CHANNELS)
                            .update_one(doc! {"_id":channel}, doc! {"$set":{"owner_sender_ids":[]}})
                            .await
                            .unwrap();
                    }
                    native_png()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let download = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        source.attachments[0].provider_ref = format!("{origin}/image");
        source.attachments[1].provider_ref = "https://untrusted.invalid/attachment".into();
        state
            .db
            .collection::<ChannelMessage>(MESSAGES)
            .replace_one(doc! {"_id":&source.id}, &source)
            .await
            .unwrap();
        crate::services::assistant_channel_media::TEST_ADAPTERS
            .lock()
            .unwrap()
            .insert(
                row.channel_bot_id.clone(),
                Arc::new(adapters::discord::DiscordAdapter::media_test_adapter(
                    &origin,
                )),
            );
        deliver(&state, &row, &key, &source).await;
        let event = state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(doc! {"event_id":&source.id})
            .await
            .unwrap()
            .unwrap();
        let conversation = event.conversation_id.as_ref().unwrap();
        let input = engine::messages(&state.db, OWNER, conversation, 100, None)
            .await
            .unwrap()
            .into_iter()
            .find(|m| m.media_bound.is_some())
            .unwrap();
        let bound = input.media_bound.as_ref().unwrap();
        assert_eq!(input.attachments.len(), usize::from(!revoke));
        assert_eq!(bound.failed.len(), if revoke { 2 } else { 1 });
        assert_eq!(
            calls.lock().await.len(),
            1,
            "attachment failures must not discard the text turn"
        );
        let roots = state
            .db
            .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME);
        if revoke {
            assert_eq!(
                roots.count_documents(doc! {}).await.unwrap(),
                0,
                "revocation during download must not bind or leave unbound uploads"
            );
        } else {
            let image = &input.attachments[0];
            let old = bson::DateTime::from_chrono(Utc::now() - ChronoDuration::days(366));
            roots
                .update_one(
                    doc! {"_id":&image.id},
                    doc! {"$set":{"bound_at":old,"created_at":old}},
                )
                .await
                .unwrap();
            assert!(matches!(
                uploads::owner_read(
                    &state.db,
                    &state.encryption_keys,
                    OWNER,
                    conversation,
                    &image.id
                )
                .await,
                Err(AppError::AssistantAttachmentExpired)
            ));
        }
        download.abort();
        server.abort();
    }
}

#[tokio::test]
async fn channel_media_v2_owner_group_topic_stays_out_of_home() {
    let (state, mut row, key, calls, server) =
        media_fixture("relay_v2_group_topic", "gateway").await;
    platform(&state, &mut row, "slack").await;
    enable_media(&state).await;
    crate::services::security_readiness_service::ready_test_replica(&state.db).await;
    gateway_ready(&state, &key).await;
    let provider = provider(&state, &row).await;
    let mut source = saved_source(&state, &row, &provider, "group-media", "human").await;
    source.platform_conversation_id = Some("team-channel".into());
    source.thread_id = Some("171234.23".into());
    state
        .db
        .collection::<ChannelMessage>(MESSAGES)
        .replace_one(doc! {"_id":&source.id}, &source)
        .await
        .unwrap();
    let mut body = provider_event(&row, &source);
    body["event_context"]["activity"]["conversation"]["kind"] = json!("group");
    body["event_context"]["activity"]["conversation"]["thread_id"] = json!(source.thread_id);
    body["event_context"]["activity"]["kind"]["mentions_bot"] = json!(true);
    let response = respond(&state, &key, &body, &source.id).await;
    assert_eq!(response.status(), StatusCode::OK);
    let text = body_text(response).await;
    assert!(text.contains("Here is your answer"), "{text}");
    let event = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"event_id":&source.id})
        .await
        .unwrap()
        .unwrap();
    let conversation = event.conversation_id.as_ref().unwrap();
    let agent = crate::services::assistant_team_service::ensure_nyxbot(&state.db, OWNER)
        .await
        .unwrap();
    let home = crate::services::assistant_team_service::home_thread(
        &state.db,
        &state.encryption_keys,
        &agent,
    )
    .await
    .unwrap()
    .id;
    assert_ne!(conversation, &home);
    let messages = engine::messages(&state.db, OWNER, conversation, 100, None)
        .await
        .unwrap();
    let input = messages
        .into_iter()
        .find(|m| m.media_bound.is_some())
        .unwrap();
    assert_eq!(input.attachments.len(), 2);
    assert!(
        uploads::owner_read(
            &state.db,
            &state.encryption_keys,
            OWNER,
            &home,
            &input.attachments[0].id
        )
        .await
        .is_err()
    );
    assert!(
        calls
            .lock()
            .await
            .last()
            .unwrap()
            .to_string()
            .contains("input_image")
    );
    server.abort();
}

#[tokio::test]
async fn channel_guest_media_relay_adapters_and_gateway_relay_retain_only_current_guest_inputs() {
    for name in ["discord", "slack", "whatsapp", "x", "telegram"] {
        for transport in ["direct", "gateway"] {
            let (state, mut row, key, calls, server) =
                media_fixture(&format!("guest_v2_{name}_{transport}"), transport).await;
            platform(&state, &mut row, name).await;
            super::guest_media_tests::enable_guest(&state).await;
            if transport == "gateway" {
                gateway_ready(&state, &key).await;
            }
            let provider = provider(&state, &row).await;
            let source = saved_source(&state, &row, &provider, "guest-media", "visitor").await;
            deliver(&state, &row, &key, &source).await;
            let event = state
                .db
                .collection::<NyxbotEvent>(EVENTS)
                .find_one(doc! {"event_id":&source.id})
                .await
                .unwrap()
                .unwrap();
            assert!(
                event.media.as_ref().is_some_and(|m| m.guest),
                "{name}/{transport}"
            );
            let conversation = event.conversation_id.as_ref().unwrap();
            let messages = engine::messages(&state.db, OWNER, conversation, 100, None)
                .await
                .unwrap();
            let input = messages
                .iter()
                .find(|m| !m.attachments.is_empty())
                .unwrap_or_else(|| panic!("{name}/{transport}: {:?}", messages));
            assert_eq!(
                input.attachments.len(),
                2,
                "{name}/{transport}: {:?}",
                input.media_bound.as_ref().map(|bound| bound
                    .failed
                    .iter()
                    .map(|failure| (failure.index, &failure.status))
                    .collect::<Vec<_>>())
            );
            assert_eq!(input.attachments[0].content_type, "image/png");
            assert_eq!(
                uploads::owner_read(
                    &state.db,
                    &state.encryption_keys,
                    OWNER,
                    conversation,
                    &input.attachments[0].id
                )
                .await
                .unwrap()
                .1,
                native_png()
            );
            let captured = calls.lock().await.last().unwrap().to_string();
            assert!(
                captured.contains("input_image"),
                "{name}/{transport}: {captured}"
            );
            assert!(captured.contains(crate::services::assistant_channel_media::guest::INPUT_NOTE));
            assert!(!captured.contains(&provider.uri()));
            let downloads = provider.received_requests().await.unwrap().len();
            let count = calls.lock().await.len();
            if transport == "direct" {
                assert!(callback(&state, &row, &source).await.is_success());
            } else {
                let reply = respond(&state, &key, &provider_event(&row, &source), "retry").await;
                assert_eq!(reply.status(), StatusCode::OK);
                let _ = body_text(reply).await;
                let alias =
                    crate::services::nyxbot_dispatch_identity::dispatch_key(&row.id, "retry");
                let winner = state
                    .db
                    .collection::<NyxbotEvent>(EVENTS)
                    .find_one(doc! {"dispatch_identity.keys":alias})
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    winner.id, event.id,
                    "{name}/{transport}: retry alias must resolve the admitted media event"
                );
            }
            assert_eq!(calls.lock().await.len(), count);
            assert_eq!(provider.received_requests().await.unwrap().len(), downloads);
            engine::delete(&state.db, OWNER, conversation)
                .await
                .unwrap();
            assert_eq!(
                state
                    .db
                    .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
                    .count_documents(doc! {"conversation_id":conversation})
                    .await
                    .unwrap(),
                0
            );
            server.abort();
        }
    }
}

#[tokio::test]
async fn whatsapp_relay_delivered_reply_releases_chat_for_next_text() {
    Box::pin(whatsapp_relay_turns()).await;
}

async fn whatsapp_relay_turns() {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let (mut state, calls, upstream) = setup("wa_relay_turns").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    state.config.base_url = format!("http://{}", listener.local_addr().unwrap());
    let (mut row, _) = channel(&state, "direct").await;
    row.owner_sender_ids = vec!["789".into()];
    platform(&state, &mut row, "whatsapp").await;
    let router = Router::new()
        .route("/api/v1/nyxbot/relay/{id}", post(relay_callback))
        .with_state(state.clone());
    let callback = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    state.db.collection::<bson::Document>("api_keys")
        .update_one(doc!{"_id":&row.route_api_key_id},doc!{"$set":{"callback_url":format!("{}/api/v1/nyxbot/relay/{}",state.config.base_url,row.id)}}).await.unwrap();
    set_bot_secrets(
        &state,
        &row,
        doc! {},
        &[
            (BotSecret::AppSecret, b"test-app-secret"),
            (BotSecret::Token, b"test-bot-token"),
        ],
    )
    .await;
    let graph = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/123/messages"))
        .respond_with(|_: &wiremock::Request| {
            ResponseTemplate::new(200)
                .set_body_json(json!({"messages":[{"id":format!("wamid.{}",Uuid::new_v4())}]}))
        })
        .mount(&graph)
        .await;
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .insert(
            row.channel_bot_id.clone(),
            Arc::new(WhatsAppGraph(graph.uri())),
        );

    let send = async |value: Value| {
        let body = serde_json::to_vec(
            &json!({"object":"whatsapp_business_account","entry":[{"id":"456","changes":[
            {"field":"messages","value":value}]}]}),
        )
        .unwrap();
        let mut mac = Hmac::<Sha256>::new_from_slice(b"test-app-secret").unwrap();
        mac.update(&body);
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-hub-signature-256",
            format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
                .parse()
                .unwrap(),
        );
        Box::pin(
            crate::handlers::channel_webhooks::handle_webhook_inner_with_producers(
                &state,
                &row.channel_bot_id,
                "whatsapp",
                &headers,
                &body,
                None,
            ),
        )
        .await
        .unwrap();
    };
    let mut chat_id = None;
    let mut turns = std::collections::HashSet::new();
    for (index, text) in ["First question", "Second question"]
        .into_iter()
        .enumerate()
    {
        let source_id = format!("wamid.source{index}");
        send(
            json!({"messaging_product":"whatsapp","metadata":{"phone_number_id":"123"},
            "messages":[{"id":source_id,"from":"789","type":"text","text":{"body":text}}]}),
        )
        .await;
        let event = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if let Some(e) = state
                    .db
                    .collection::<NyxbotEvent>(EVENTS)
                    .find_one(doc! {"channel_id":&row.id, "delivery.state":"sent"})
                    .sort(doc! {"created_at":-1})
                    .await
                    .unwrap()
                    && !turns.contains(e.turn_id.as_ref().unwrap())
                {
                    break e;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("relay did not finish the WhatsApp send");
        let out = state
            .db
            .collection::<ChannelMessage>(MESSAGES)
            .find_one(doc! {
                "channel_bot_id":&row.channel_bot_id, "direction":"outbound"
            })
            .sort(doc! {"created_at":-1})
            .await
            .unwrap()
            .unwrap();
        let wamid = out.platform_message_id.unwrap();
        send(json!({"messaging_product":"whatsapp","metadata":{"phone_number_id":"123"},
            "statuses":[{"id":wamid,"recipient_id":"789","status":"delivered","timestamp":Utc::now().timestamp().to_string()}]})).await;
        assert!(state.db.collection::<bson::Document>(crate::models::channel_delivery::COLLECTION_NAME)
            .find_one(doc!{"bot_id":&row.channel_bot_id,"platform_message_id":&wamid,"times.delivered_at":{"$type":"date"}})
            .await.unwrap().is_some());
        let conversation = engine::get(&state.db, OWNER, event.conversation_id.as_deref().unwrap())
            .await
            .unwrap();
        assert_eq!(
            chat_id.get_or_insert(conversation.id.clone()),
            &conversation.id
        );
        assert!(conversation.active_turn.is_none());
        assert!(turns.insert(event.turn_id.unwrap()));
    }
    assert_eq!(calls.lock().await.len(), 2);
    assert_eq!(graph.received_requests().await.unwrap().len(), 2);
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .remove(&row.channel_bot_id);
    callback.abort();
    upstream.abort();
    state.db.drop().await.unwrap();
}
