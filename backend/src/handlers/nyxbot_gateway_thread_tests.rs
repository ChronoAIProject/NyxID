use super::*;
use crate::{
    models::{
        channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage},
        channel_thread::{ChannelThreadFacts, ThreadAddress, ThreadKind, ThreadSenderKind},
    },
    services::{
        channel_platform::InboundMessage, channel_thread_follow_service as follow,
        channel_thread_service as threads,
    },
};
use futures::TryStreamExt;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::*};

struct Fixture {
    state: AppState,
    row: NyxbotChannel,
    key: String,
    calls: Calls,
    model: tokio::task::JoinHandle<()>,
    gateway: MockServer,
    platform: MockServer,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.model.abort();
        thread_follow::TEST_ADAPTERS
            .lock()
            .unwrap()
            .remove(&self.row.channel_bot_id);
    }
}
impl Fixture {
    async fn new(name: &str) -> Self {
        let (state, calls, model) = setup(name).await;
        let (mut row, key) = channel(&state, "gateway").await;
        row.platform = "lark".into();
        row.owner_sender_ids = vec!["human".into()];
        row.gateway_channel_id = Some("cmaeg1.ch.test".into());
        row.gateway_bot_id = Some("ou_bot".into());
        row.gateway_version = Some(1);
        row.gateway_bot_mentions.supported = true;
        row.gateway_bot_mentions.enabled = true;
        row.gateway_bot_mentions.checked_at = Some(Utc::now());
        row.gateway_threads.supported = true;
        row.gateway_threads.version = Some(1);
        row.gateway_threads.enabled = true;
        row.gateway_threads.checked_at = Some(Utc::now());
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .replace_one(doc! {"_id":&row.id}, &row)
            .await
            .unwrap();
        let mut bot = bot_doc("lark", "Thread helper");
        bot.insert("_id", &row.channel_bot_id);
        bot.insert("platform_bot_id", "ou_bot");
        bot.insert(
            "bot_token_encrypted",
            bson::Binary {
                subtype: bson::spec::BinarySubtype::Generic,
                bytes: state.encryption_keys.encrypt(b"app:secret").await.unwrap(),
            },
        );
        state
            .db
            .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
            .replace_one(
                doc! {"_id": bot.get_str("_id").unwrap().to_owned()},
                bot.clone(),
            )
            .upsert(true)
            .await
            .unwrap();
        state
            .db
            .collection::<bson::Document>("api_keys")
            .update_one(
                doc! {"_id":&row.route_api_key_id},
                doc! {"$set":{"callback_url":"https://example.test/callback"}},
            )
            .await
            .unwrap();
        // Reuse channel()'s real route so native validation sees one active
        // default whose identity matches the source, rather than two defaults.
        assert_eq!(state.db.collection::<bson::Document>("channel_conversations").replace_one(
            doc! {"_id": &row.route_id},
            doc! {"_id":&row.route_id,"user_id":OWNER,"channel_bot_id":&row.channel_bot_id,"platform":"lark","platform_conversation_id":"*","platform_conversation_type":"group","agent_api_key_id":&row.route_api_key_id,"default_agent":true,"is_active":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}
        ).await.unwrap().matched_count, 1);
        state.db.collection::<bson::Document>("feature_flag_overrides").insert_one(doc! {"_id":"follow-flag","org_user_id":bson::Bson::Null,"flag_key":crate::services::feature_flag_service::NYXBOT_THREAD_FOLLOW_FLAG_KEY,"target_kind":"user","target_key":OWNER,"enabled":true,"updated_by":"admin","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
        let gateway = MockServer::start().await;
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = Uuid::new_v4().to_string();
        service.slug = "cmaeg".into();
        service.requires_user_credential = false;
        service.service_category = "internal".into();
        service.base_url = gateway.uri();
        state
            .db
            .collection::<DownstreamService>(SERVICES)
            .insert_one(service)
            .await
            .unwrap();
        Mock::given(method("PUT")).respond_with(|r:&wiremock::Request| {
            let body:Value=r.body_json().unwrap();
            ResponseTemplate::new(200).set_body_json(json!({"version":body["expected_version"].as_i64().unwrap()+1,"definition":body}))
        }).mount(&gateway).await;
        let platform = MockServer::start().await;
        Mock::given(path("/open-apis/auth/v3/tenant_access_token/internal"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"code":0,"tenant_access_token":"test-token","expire":7200}),
                ),
            )
            .mount(&platform)
            .await;
        thread_follow::TEST_ADAPTERS.lock().unwrap().insert(
            row.channel_bot_id.clone(),
            Arc::new(
                crate::services::channel_adapters::lark::LarkFamilyAdapter::media_test_adapter(
                    &platform.uri(),
                    "lark",
                ),
            ),
        );
        assert_eq!(
            put_binding(
                State(state.clone()),
                Path("bnd_chats".into()),
                bearer(&key),
                Json(binding_body(&key, OWNER))
            )
            .await
            .status(),
            StatusCode::OK
        );
        ensure_partition(&state, &key, PARTITION).await;
        Self {
            state,
            row,
            key,
            calls,
            model,
            gateway,
            platform,
        }
    }
    async fn input(
        &self,
        text: &str,
        address: ThreadAddress,
        others: bool,
        metadata: bool,
    ) -> (Value, String) {
        let id = Uuid::new_v4().to_string();
        let native = format!("om_{}", Uuid::new_v4().simple());
        let facts = ChannelThreadFacts {
            version: 1,
            kind: ThreadKind::Native,
            chat_id: "oc_chat".into(),
            message_id: native.clone(),
            root_id: Some("om_root".into()),
            native_thread_id: Some("omt_thread".into()),
            sender_kind: ThreadSenderKind::Human,
            address,
            mentions_others: others,
            ..Default::default()
        };
        let inbound = InboundMessage {
            embedded_attachments: Vec::new(),
            embedded_attachments_omitted: 0,
            platform_message_id: native,
            conversation_id: "oc_chat".into(),
            conversation_type: "group".into(),
            sender_platform_id: "human".into(),
            sender_display_name: None,
            content_type: "text".into(),
            text: Some(text.into()),
            attachments: vec![],
            reply_to_platform_message_id: None,
            thread_id: Some("omt_thread".into()),
            raw_data: json!({}),
        };
        let source = crate::services::channel_relay_service::inbound_metadata(
            &self.row.channel_bot_id,
            self.row.route_id.as_deref().unwrap(),
            OWNER,
            "lark",
            &inbound,
            &self.row.route_api_key_id,
            &id,
        );
        self.state
            .db
            .collection::<ChannelMessage>(MESSAGES)
            .insert_one(&source)
            .await
            .unwrap();
        let mut body = event(text, "human", &id);
        let activity = &mut body["event_context"]["activity"];
        activity["source"] =
            json!({"type":"nyxid_relay","platform":"lark","route_id":self.row.route_id});
        activity["conversation"] = json!({"id":"oc_chat","kind":"group","thread_id":"omt_thread"});
        activity["kind"]["mentions_bot"] = json!(address == ThreadAddress::Mention);
        if metadata {
            activity["thread"] = serde_json::to_value(facts).unwrap();
        }
        (body, id)
    }
    async fn send(&self, body: &Value, id: &str) -> String {
        let response = Box::pin(respond(&self.state, &self.key, body, id)).await;
        assert_eq!(response.status(), StatusCode::OK);
        body_text(response).await
    }
    async fn children(&self) -> Vec<NyxbotThread> {
        self.state
            .db
            .collection(THREADS)
            .find(doc! {"channel_id":&self.row.id,"record_scope":follow::SCOPE})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn gateway_follow_shares_child_filters_other_mentions_and_retains_context() {
    let f = Box::pin(Fixture::new("nyx_gateway_follow")).await;
    let (body, id) = f.input("start", ThreadAddress::Mention, false, true).await;
    let answer = f.send(&body, &id).await;
    assert!(answer.contains("Here is your answer"), "{answer}");
    assert!(answer.contains("\"thread_reply\":true"), "{answer}");
    let children = f.children().await;
    assert_eq!(children.len(), 1);
    let conversation = children[0].conversation_id.as_deref().unwrap();
    settle(&f.state, conversation).await;
    let row = load_channel(&f.state, OWNER, &f.row.id).await.unwrap();
    assert_eq!(row.gateway_threads.follow_chat_ids, ["oc_chat"]);
    let requests = f.gateway.received_requests().await.unwrap();
    assert!(requests.iter().any(|r| {
        r.body_json::<Value>()
            .unwrap()
            .pointer("/reply/thread_contract/follow_chat_ids")
            == Some(&json!(["oc_chat"]))
    }));
    let (quiet, quiet_id) = f
        .input(
            "addressed to someone else",
            ThreadAddress::NotAddressed,
            true,
            true,
        )
        .await;
    assert!(
        !f.send(&quiet, &quiet_id)
            .await
            .contains("Here is your answer")
    );
    assert_eq!(f.calls.lock().await.len(), 1);
    let retained = f
        .state
        .db
        .collection::<ChannelMessage>(MESSAGES)
        .find_one(doc! {"_id":&quiet_id})
        .await
        .unwrap()
        .unwrap();
    assert!(retained.thread_context.unwrap().mentions_others);
    let (mut plain, id) = f
        .input(
            "ordinary follow-up",
            ThreadAddress::NotAddressed,
            false,
            true,
        )
        .await;
    let guest_partition = "conv_00000000000000000000000000000009";
    ensure_partition(&f.state, &f.key, guest_partition).await;
    plain["conversation"] = json!(guest_partition);
    plain["event_context"]["conversation_id"] = json!(guest_partition);
    plain["event_context"]["activity"]["actor"]["id"] = json!("guest");
    f.state
        .db
        .collection::<ChannelMessage>(MESSAGES)
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"sender_platform_id":"guest"}},
        )
        .await
        .unwrap();
    assert!(f.send(&plain, &id).await.contains("Here is your answer"));
    assert_eq!(
        f.children().await[0].conversation_id.as_deref(),
        Some(conversation)
    );
    assert_eq!(f.calls.lock().await.len(), 2);
    assert!(
        f.calls.lock().await[1].to_string().contains(
            quiet["event_context"]["activity"]["thread"]["message_id"]
                .as_str()
                .unwrap()
        )
    );
    assert!(
        !f.calls.lock().await[1]
            .to_string()
            .contains("addressed to someone else")
    );
    settle(&f.state, conversation).await;
    let (both, id) = f
        .input("bot and others", ThreadAddress::Mention, false, true)
        .await;
    assert!(f.send(&both, &id).await.contains("Here is your answer"));
    settle(&f.state, conversation).await;
    // Broadcasts are ordinary followed messages, never bot mentions.
    let (broadcast, id) = f
        .input("broadcast", ThreadAddress::NotAddressed, false, true)
        .await;
    assert!(
        f.send(&broadcast, &id)
            .await
            .contains("Here is your answer")
    );
    settle(&f.state, conversation).await;
    // Widened admission alone cannot make missing metadata address the bot.
    let before = f.calls.lock().await.len();
    let (missing, id) = f
        .input(
            "missing metadata",
            ThreadAddress::NotAddressed,
            false,
            false,
        )
        .await;
    assert!(!f.send(&missing, &id).await.contains("Here is your answer"));
    assert_eq!(f.calls.lock().await.len(), before);
}

#[tokio::test]
async fn gateway_without_negotiation_or_metadata_keeps_legacy_thread_behavior() {
    let f = Box::pin(Fixture::new("nyx_gateway_legacy")).await;
    f.state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$unset":{"gateway_threads":""}},
        )
        .await
        .unwrap();
    // A lost management acknowledgement can leave local readiness unset while
    // the gateway already admits ordinary messages with the new metadata.
    let (unacknowledged, id) = f
        .input(
            "ordinary message before acknowledgement",
            ThreadAddress::NotAddressed,
            false,
            true,
        )
        .await;
    assert!(
        !f.send(&unacknowledged, &id)
            .await
            .contains("Here is your answer")
    );
    assert!(f.calls.lock().await.is_empty());
    let (body, id) = f
        .input("legacy mention", ThreadAddress::Mention, false, false)
        .await;
    let answer = f.send(&body, &id).await;
    assert!(answer.contains("Here is your answer"), "{answer}");
    assert!(!answer.contains("thread_reply"));
    assert!(f.children().await.is_empty());
    assert!(f.gateway.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn gateway_no_follow_or_private_or_missing_metadata_performs_zero_follow_commands() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let commands = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = commands.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "nyx_gw_fast",
        EventHandler::callback(move |e| {
            if let CommandEvent::Started(e) = e {
                sink.lock().unwrap().push(e.command_name);
            }
        }),
    )
    .await;
    let state = crate::test_utils::test_app_state(db);
    let (mut row, _) = channel(&state, "gateway").await;
    row.platform = "lark".into();
    row.gateway_threads.version = Some(1);
    row.gateway_threads.supported = true;
    let mut activity =
        json!({"conversation":{"kind":"group","id":"oc_chat"},"thread":{"version":1}});
    commands.lock().unwrap().clear();
    assert!(
        Box::pin(thread_follow::gateway_inbound(
            &state, &row, &activity, "text", "event"
        ))
        .await
        .unwrap()
        .is_none()
    );
    row.gateway_threads.enabled = true;
    row.gateway_threads.excluded_chat_ids.push("oc_chat".into());
    assert!(
        Box::pin(thread_follow::gateway_inbound(
            &state, &row, &activity, "text", "event"
        ))
        .await
        .unwrap()
        .is_none()
    );
    row.gateway_threads.excluded_chat_ids.clear();
    activity["conversation"]["kind"] = json!("private");
    assert!(
        Box::pin(thread_follow::gateway_inbound(
            &state, &row, &activity, "text", "event"
        ))
        .await
        .unwrap()
        .is_none()
    );
    activity["conversation"]["kind"] = json!("group");
    activity["thread"] = Value::Null;
    assert!(
        Box::pin(thread_follow::gateway_inbound(
            &state, &row, &activity, "text", "event"
        ))
        .await
        .unwrap()
        .is_none()
    );
    assert!(commands.lock().unwrap().is_empty());
    assert!(!threads::gateway::candidate(&row, &activity));
}

#[tokio::test]
async fn gateway_reply_uses_original_native_root_and_rejects_unbound_sources() {
    use crate::handlers::channel_relay::{
        AsyncReplyBody, AsyncReplyRequest, async_reply_with_test_adapter,
    };
    let f = Box::pin(Fixture::new("nyx_gw_reply")).await;
    let (body, id) = f.input("start", ThreadAddress::Mention, false, true).await;
    assert!(f.send(&body, &id).await.contains("Here is your answer"));
    Mock::given(method("POST"))
        .and(path("/open-apis/im/v1/messages/om_root/reply"))
        .and(body_partial_json(json!({"reply_in_thread":true})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"code":0,"data":{"message_id":"om_answer"}})),
        )
        .expect(1)
        .mount(&f.platform)
        .await;
    let token = crate::crypto::jwt::generate_relay_reply_token(
        &f.state.jwt_keys,
        &f.state.config,
        &f.row.route_api_key_id,
        f.row.route_id.as_deref().unwrap(),
        &id,
        "lark",
    )
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
    let adapter = thread_follow::TEST_ADAPTERS
        .lock()
        .unwrap()
        .get(&f.row.channel_bot_id)
        .cloned()
        .unwrap();
    let result = Box::pin(async_reply_with_test_adapter(
        &f.state,
        &headers,
        AsyncReplyRequest {
            message_id: id.clone(),
            thread_reply: true,
            reply: AsyncReplyBody {
                text: Some("native answer".into()),
                metadata: None,
                attachments: vec![],
            },
        },
        adapter.as_ref(),
    ))
    .await
    .unwrap();
    assert_eq!(result.0.platform_message_id.as_deref(), Some("om_answer"));
    let stored = f
        .state
        .db
        .collection::<ChannelMessage>(MESSAGES)
        .find_one(doc! {"_id":result.0.message_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.thread_context.unwrap().root_id.as_deref(),
        Some("om_root")
    );
    let child = f.children().await.remove(0);
    settle(&f.state, child.conversation_id.as_deref().unwrap()).await;
    let (mut replying, reply_id) = f
        .input(
            "reply to bot while mentioning others",
            ThreadAddress::NotAddressed,
            true,
            true,
        )
        .await;
    replying["event_context"]["activity"]["thread"]["parent_message_id"] = json!("om_answer");
    f.state
        .db
        .collection::<ChannelMessage>(MESSAGES)
        .update_one(
            doc! {"_id":&reply_id},
            doc! {"$set":{"reply_to_platform_message_id":"om_answer"}},
        )
        .await
        .unwrap();
    assert!(
        f.send(&replying, &reply_id)
            .await
            .contains("Here is your answer")
    );
    let (_, unbound) = f
        .input("no metadata", ThreadAddress::Mention, false, false)
        .await;
    let token = crate::crypto::jwt::generate_relay_reply_token(
        &f.state.jwt_keys,
        &f.state.config,
        &f.row.route_api_key_id,
        f.row.route_id.as_deref().unwrap(),
        &unbound,
        "lark",
    )
    .unwrap();
    headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
    assert!(
        Box::pin(async_reply_with_test_adapter(
            &f.state,
            &headers,
            AsyncReplyRequest {
                message_id: unbound,
                thread_reply: true,
                reply: AsyncReplyBody {
                    text: Some("must not escape".into()),
                    metadata: None,
                    attachments: vec![]
                }
            },
            adapter.as_ref()
        ))
        .await
        .is_err()
    );
}

#[tokio::test]
async fn gateway_negotiation_requires_advertisement_and_echo_and_backs_off_legacy() {
    let f = Box::pin(Fixture::new("nyx_gw_negotiate")).await;
    f.state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$unset":{"gateway_threads":""}, "$set":{"gateway_bot_mentions.supported":false,"gateway_bot_mentions.enabled":false,"gateway_bot_mentions.checked_at":bson::DateTime::now()}},
        )
        .await
        .unwrap();
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"version":1})))
        .expect(1)
        .mount(&f.gateway)
        .await;
    chats::sync_gateway_groups(&f.state, &f.row, true)
        .await
        .unwrap();
    let legacy = load_channel(&f.state, OWNER, &f.row.id).await.unwrap();
    assert!(!threads::gateway::negotiated(&legacy));
    assert!(legacy.gateway_no_progress);
    chats::sync_gateway_groups(&f.state, &legacy, true)
        .await
        .unwrap();
    let requests = f.gateway.received_requests().await.unwrap();
    // Unsupported thread capability is probed once and backs off, while an
    // independent, echoed policy update reconciles quiet delivery exactly once.
    assert_eq!(requests.iter().filter(|r| r.method == "GET").count(), 1);
    let updates = requests
        .iter()
        .filter(|r| r.method == "PUT")
        .collect::<Vec<_>>();
    assert_eq!(updates.len(), 1);
    assert_eq!(
        updates[0].body_json::<Value>().unwrap()["reply"]["ack_after_ms"],
        0
    );
    f.gateway.reset().await;
    f.state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$unset":{"gateway_threads.checked_at":""}, "$set":{"gateway_no_progress":false}},
        )
        .await
        .unwrap();
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"version":1,"thread_contract_versions":[1]})),
        )
        .mount(&f.gateway)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"version":2})))
        .mount(&f.gateway)
        .await;
    assert_eq!(
        chats::sync_gateway_groups(&f.state, &legacy, true)
            .await
            .unwrap(),
        Some("gateway_thread_contract_unavailable")
    );
    let unconfirmed = load_channel(&f.state, OWNER, &f.row.id).await.unwrap();
    assert!(!threads::gateway::negotiated(&unconfirmed));
    assert!(!unconfirmed.gateway_no_progress);
    f.gateway.reset().await;
    Mock::given(method("PUT"))
        .respond_with(|r: &wiremock::Request| {
            ResponseTemplate::new(200)
                .set_body_json(json!({"version":3,"definition":r.body_json::<Value>().unwrap()}))
        })
        .mount(&f.gateway)
        .await;
    assert!(
        chats::sync_gateway_groups(&f.state, &legacy, true)
            .await
            .unwrap()
            .is_none()
    );
    let confirmed = load_channel(&f.state, OWNER, &f.row.id).await.unwrap();
    assert!(threads::gateway::negotiated(&confirmed));
    assert!(confirmed.gateway_no_progress);
}

#[tokio::test]
async fn nyxbot_followed_thread_images_use_native_components_before_direct_or_gateway_text() {
    use crate::models::{
        assistant_attachment::AssistantAttachment, assistant_conversation::TurnAttachment,
    };
    for gateway in [false, true] {
        let f = Box::pin(Fixture::new("nyxbot_follow_images")).await;
        let (body, id) = f.input("start", ThreadAddress::Mention, false, true).await;
        assert!(f.send(&body, &id).await.contains("Here is your answer"));
        let child = f.children().await.remove(0);
        let mut row = engine::get(
            &f.state.db,
            OWNER,
            child.conversation_id.as_deref().unwrap(),
        )
        .await
        .unwrap();
        let origin = row.channel.clone().unwrap();
        let turn_id = Uuid::new_v4().to_string();
        row.active_turn=Some(bson::from_document(doc! {"turn_id":&turn_id,"origin":"event","started_at":bson::DateTime::now(),"stop_requested":false}).unwrap());
        if !gateway {
            f.state
                .db
                .collection::<bson::Document>(CHANNELS)
                .update_one(doc! {"_id":&f.row.id}, doc! {"$set":{"transport":"direct"}})
                .await
                .unwrap();
        }
        let mut attachments = Vec::new();
        for _ in 0..2 {
            let attachment = AssistantAttachment {
                secret_context_versions: Default::default(),
                id: Uuid::new_v4().to_string(),
                origin: "tool".into(),
                user_id: OWNER.into(),
                conversation_id: row.id.clone(),
                turn_id: turn_id.clone(),
                content_type: "image/png".into(),
                size: 6,
                data_encrypted: f.state.encryption_keys.encrypt(b"pixels").await.unwrap(),
                created_at: Utc::now(),
            };
            f.state
                .db
                .collection::<AssistantAttachment>(
                    crate::models::assistant_attachment::COLLECTION_NAME,
                )
                .insert_one(&attachment)
                .await
                .unwrap();
            attachments.push(TurnAttachment {
                id: attachment.id,
                label: "private label".into(),
                content_type: "image/png".into(),
                size: 6,
                origin: "tool".into(),
                pages: None,
                chat_delivery: None,
                image_input: None,
                image_inspection: None,
            });
        }
        Mock::given(path("/open-apis/im/v1/images"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code":0,"data":{"image_key":"image"}})),
            )
            .expect(2)
            .mount(&f.platform)
            .await;
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = count.clone();
        Mock::given(path("/open-apis/im/v1/messages/om_root/reply")).and(body_partial_json(json!({"reply_in_thread":true})))
            .respond_with(move |r:&wiremock::Request| {
                let body:Value=r.body_json().unwrap();
                if body["msg_type"]=="image" {seen.fetch_add(1,std::sync::atomic::Ordering::SeqCst);}else{assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst),2);}
                assert!(!body.to_string().contains("private label"));
                ResponseTemplate::new(200).set_body_json(json!({"code":0,"data":{"message_id":format!("om_sent_{}",Uuid::new_v4().simple())}}))
            }).expect(if gateway{2}else{3}).mount(&f.platform).await;
        let seen = count.clone();
        Mock::given(path("/events/cmaeg1.ev.sealed-reference/replies"))
            .respond_with(move |r: &wiremock::Request| {
                assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 2);
                let body: Value = r.body_json().unwrap();
                assert_eq!(body["thread_reply"], true);
                assert_eq!(body["text"], "event result");
                ResponseTemplate::new(200).set_body_json(json!({"ok":true}))
            })
            .expect(u64::from(gateway))
            .mount(&f.gateway)
            .await;
        deliver_to(
            &f.state,
            &row,
            &origin,
            "event result",
            Some(&images::ImageSendPlan { attachments }),
        )
        .await;
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
        let requests = f.platform.received_requests().await.unwrap();
        let replies: Vec<_> = requests
            .iter()
            .filter(|request| request.url.path().ends_with("/reply"))
            .collect();
        for request in replies {
            assert_eq!(
                request.url.path(),
                "/open-apis/im/v1/messages/om_root/reply"
            );
            let body: Value = request.body_json().unwrap();
            assert_eq!(body["reply_in_thread"], true);
            if body["msg_type"] == "image" {
                let content: Value =
                    serde_json::from_str(body["content"].as_str().unwrap()).unwrap();
                assert_eq!(content["image_key"], "image");
            }
        }
        assert!(
            !requests
                .iter()
                .any(|request| request.url.path() == "/open-apis/im/v1/messages")
        );
        f.platform.verify().await;
        f.gateway.verify().await;
        let claims = f
            .state
            .db
            .collection::<crate::models::nyxbot_image_delivery::NyxbotImageDelivery>(
                crate::models::nyxbot_image_delivery::COLLECTION_NAME,
            )
            .find(doc! {"conversation_id":&row.id})
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap();
        assert_eq!(claims.len(), 2);
        assert!(
            claims
                .iter()
                .all(|i| i.state == crate::models::nyxbot_image_delivery::ImageDeliveryState::Sent)
        );
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn gateway_complete_thread_reply_uses_one_token_and_reconstructs_every_part() {
    use crate::handlers::channel_relay::{
        AsyncReplyBody, AsyncReplyRequest, async_reply_with_test_adapter,
    };
    let f = Box::pin(Fixture::new("nyx_gw_complete_reply")).await;
    let (body, id) = f.input("start", ThreadAddress::Mention, false, true).await;
    assert!(f.send(&body, &id).await.contains("Here is your answer"));
    let counter = std::sync::atomic::AtomicUsize::new(1);
    Mock::given(method("POST"))
        .and(path("/open-apis/im/v1/messages/om_root/reply"))
        .and(body_partial_json(json!({"reply_in_thread":true})))
        .respond_with(move |_: &wiremock::Request| ResponseTemplate::new(200).set_body_json(json!({
            "code":0,"data":{"message_id":format!("om_part_{}", counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst))}
        }))).mount(&f.platform).await;
    let token = crate::crypto::jwt::generate_relay_reply_token(
        &f.state.jwt_keys,
        &f.state.config,
        &f.row.route_api_key_id,
        f.row.route_id.as_deref().unwrap(),
        &id,
        "lark",
    )
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
    let adapter = thread_follow::TEST_ADAPTERS
        .lock()
        .unwrap()
        .get(&f.row.channel_bot_id)
        .cloned()
        .unwrap();
    let text = format!(
        "{}\n\nhttps://example.com/complete-link",
        "界🦉\\*\n\n".repeat(8000)
    );
    let request = || AsyncReplyRequest {
        message_id: id.clone(),
        thread_reply: true,
        reply: AsyncReplyBody {
            text: Some(text.clone()),
            metadata: None,
            attachments: vec![],
        },
    };
    let result = Box::pin(async_reply_with_test_adapter(
        &f.state,
        &headers,
        request(),
        adapter.as_ref(),
    ))
    .await
    .unwrap();
    assert!(
        result
            .0
            .platform_message_id
            .unwrap()
            .starts_with("om_part_")
    );
    let requests: Vec<_> = f
        .platform
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == "/open-apis/im/v1/messages/om_root/reply")
        .collect();
    assert!(requests.len() > 1);
    let mut reconstructed = String::new();
    for request in &requests {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["reply_in_thread"], true);
        let content = body["content"].as_str().unwrap();
        assert!(content.len() <= 30 * 1024);
        let content: Value = serde_json::from_str(content).unwrap();
        reconstructed.push_str(content["text"].as_str().unwrap());
    }
    assert_eq!(reconstructed, text);
    assert!(
        Box::pin(async_reply_with_test_adapter(
            &f.state,
            &headers,
            request(),
            adapter.as_ref()
        ))
        .await
        .is_err()
    );
    let record = f
        .state
        .db
        .collection::<bson::Document>(crate::models::channel_reply_delivery::COLLECTION_NAME)
        .find_one(doc! {"inbound_message_id": &id})
        .await
        .unwrap()
        .unwrap();
    assert!(record.get_bool("complete").unwrap());
    assert_eq!(record.get_array("parts").unwrap().len(), requests.len());
}

#[tokio::test]
async fn external_bot_mention_in_followed_thread_is_a_guest_and_spends_the_parent_budget() {
    let f = Box::pin(Fixture::new("external_bot_follow")).await;
    let (human, id) = f.input("start", ThreadAddress::Mention, false, true).await;
    assert!(f.send(&human, &id).await.contains("Here is your answer"));
    let child = f.children().await.remove(0);
    settle(&f.state, child.conversation_id.as_deref().unwrap()).await;
    // A delivered app actor, not a sender who has linked an OAuth account.
    let (mut body, id) = f
        .input("exact mention", ThreadAddress::Mention, false, true)
        .await;
    let activity = &mut body["event_context"]["activity"];
    activity["actor"]["kind"] = json!("bot");
    activity["actor"]["id"] = json!("ou_foreign_bot");
    activity["thread"]["sender_kind"] = json!("bot");
    f.state.db.collection::<ChannelMessage>(MESSAGES).update_one(doc! {"_id": &id},
        doc! {"$set":{"sender_platform_id":"ou_foreign_bot","thread_context":bson::to_bson(&activity["thread"]).unwrap()}}).await.unwrap();
    let text = f.send(&body, &id).await;
    assert!(text.contains("Here is your answer"), "{text}");
    assert!(text.contains("\"thread_reply\":true"), "{text}");
    let children = f.children().await;
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].id, child.id);
    assert_eq!(
        children[0].follow.follow_expires_at, child.follow.follow_expires_at,
        "a bot cannot renew follow"
    );
    let conversation = crate::services::assistant_nyxagent::get(
        &f.state.db,
        OWNER,
        children[0].conversation_id.as_deref().unwrap(),
    )
    .await
    .unwrap();
    assert!(conversation.guest_turn);
    assert!(
        conversation
            .channel
            .as_ref()
            .unwrap()
            .thread
            .as_ref()
            .unwrap()
            .guest
    );
    assert_eq!(f.calls.lock().await.len(), 2);
}

#[tokio::test]
async fn channel_media_relay_and_gateway_follow_share_one_native_barrier_and_authority() {
    use crate::{
        models::channel_message::StoredAttachment,
        services::channel_media_authority::{self as authority, MediaRequest},
    };
    // Construct large fixture and handler futures outside the test's polling
    // frame; inline Box::pin still reserves construction temporaries on its stack.
    fn fixture() -> futures::future::BoxFuture<'static, Fixture> {
        Box::pin(Fixture::new("media_follow_barrier"))
    }
    fn gateway_response<'a>(
        f: &'a Fixture,
        body: &'a Value,
        id: &'a str,
    ) -> futures::future::BoxFuture<'a, Response> {
        Box::pin(respond(&f.state, &f.key, body, id))
    }
    fn direct_response(
        f: &Fixture,
        headers: HeaderMap,
        body: Bytes,
    ) -> futures::future::BoxFuture<'static, Response> {
        Box::pin(relay_callback(
            State(f.state.clone()),
            Path(f.row.id.clone()),
            headers,
            body,
        ))
    }
    for gateway in [false, true] {
        let mut f = fixture().await;
        f.state
            .db
            .collection::<bson::Document>("feature_flag_overrides")
            .insert_one(doc! {
                "_id":Uuid::new_v4().to_string(),"org_user_id":bson::Bson::Null,
                "flag_key":crate::services::feature_flag_service::ASSISTANT_CHANNEL_MEDIA_FLAG_KEY,
                "target_kind":"user","target_key":OWNER,"enabled":true,"updated_by":"test",
                "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
            })
            .await
            .unwrap();
        if !gateway {
            f.row.transport = "direct".into();
            f.state
                .db
                .collection::<NyxbotChannel>(CHANNELS)
                .replace_one(doc! {"_id":&f.row.id}, &f.row)
                .await
                .unwrap();
        }
        Mock::given(path("/open-apis/im/v1/messages/om_root/reply"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code":0,"data":{"message_id":"om_reply"}})),
            )
            .mount(&f.platform)
            .await;
        let (mut a, id_a) = f
            .input("receipt with caption", ThreadAddress::Mention, false, true)
            .await;
        let (mut b, id_b) = f
            .input("receipt with caption", ThreadAddress::Mention, false, true)
            .await;
        let provider = super::media_tests::ingestion_provider(
            &f.state,
            &f.row,
            super::media_tests::native_png(),
        )
        .await;
        for (body, id) in [(&mut a, &id_a), (&mut b, &id_b)] {
            body["event_context"]["activity"]["kind"]["attachments"] =
                json!([{"kind":"image","source":{"type":"nyxid_relay","index":0}}]);
            body["event_context"]["activity"]["thread"]["message_id"] = json!("om_native_receipt");
            let item = StoredAttachment {
                content_type: "image".into(),
                provider_ref: format!(
                    "{}/open-apis/im/v1/messages/om_native_receipt/resources/img_receipt?type=image",
                    provider.uri()
                ),
                platform_message_id: Some("om_native_receipt".into()),
                image_key: Some("img_receipt".into()),
                file_key: None,
                filename: None,
                mime_type: None,
                size_bytes: Some(42),
            };
            let facts: ChannelThreadFacts =
                serde_json::from_value(body["event_context"]["activity"]["thread"].clone())
                    .unwrap();
            f.state.db.collection::<ChannelMessage>(MESSAGES).update_one(doc!{"_id":id},doc!{"$set":{
                "platform_message_id":"om_native_receipt","content_type":"image","attachments":[bson::to_bson(&item).unwrap()],"thread_context":bson::to_bson(&facts).unwrap()
            }}).await.unwrap();
        }
        if gateway {
            let (a, b) = tokio::join!(
                gateway_response(&f, &a, &id_a),
                gateway_response(&f, &b, &id_b)
            );
            assert_eq!(a.status(), StatusCode::OK);
            assert_eq!(b.status(), StatusCode::OK);
            let _ = tokio::join!(body_text(a), body_text(b));
        } else {
            let signed = |id: &str| {
                let payload = json!({"message_id":id,"correlation_id":format!("jti-{id}"),"platform":"lark",
                    "conversation":{"platform_id":"oc_chat","type":"group"},"thread_id":"omt_thread",
                    "sender":{"platform_id":"human"},"content":{"type":"image","text":"receipt with caption"},
                    "raw_platform_data":{"event":{"sender":{"sender_type":"user","sender_id":{"open_id":"human"}}}}});
                let bytes = serde_json::to_vec(&payload).unwrap();
                let token = crate::crypto::jwt::generate_relay_callback_token(
                    &f.state.jwt_keys,
                    &f.state.config,
                    &format!("jti-{id}"),
                    &f.row.route_api_key_id,
                    id,
                    "lark",
                    &sha256_hex(&bytes),
                )
                .unwrap();
                let mut headers = HeaderMap::new();
                headers.insert("x-nyxid-callback-token", token.parse().unwrap());
                (headers, Bytes::from(bytes))
            };
            let (ha, ba) = signed(&id_a);
            let (hb, bb) = signed(&id_b);
            let (a, b) = tokio::join!(direct_response(&f, ha, ba), direct_response(&f, hb, bb));
            assert_eq!(a.status(), StatusCode::ACCEPTED);
            assert_eq!(b.status(), StatusCode::ACCEPTED);
            wait_for_calls(&f.calls, 1).await;
        }
        assert_eq!(f.calls.lock().await.len(), 1, "gateway={gateway}");
        assert!(f.calls.lock().await[0].to_string().contains("input_image"));
        let events: Vec<NyxbotEvent> = f
            .state
            .db
            .collection(EVENTS)
            .find(doc! {"channel_id":&f.row.id})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert!(event.media.is_some());
        assert!(event.turn_id.is_some());
        let loser = if event.event_id == id_a { &id_b } else { &id_a };
        assert_eq!(
            f.state
                .db
                .collection::<ChannelMessage>(MESSAGES)
                .find_one(doc! {"_id":loser})
                .await
                .unwrap()
                .unwrap()
                .duplicate_of_media_key
                .as_deref(),
            Some(event.media.as_ref().unwrap().key.as_str())
        );
        let conversation = engine::get(
            &f.state.db,
            OWNER,
            event.conversation_id.as_deref().unwrap(),
        )
        .await
        .unwrap();
        let origin = conversation.channel.unwrap();
        let request = MediaRequest {
            channel: load_channel(&f.state, OWNER, &f.row.id).await.unwrap(),
            source_message_id: event.event_id.clone(),
            chat_id: "oc_chat".into(),
            sender_id: "human".into(),
            native_thread_id: Some("omt_thread".into()),
            agent_id: Some(origin.thread.as_ref().unwrap().agent_id.clone()),
            origin: Some(origin.clone()),
            conversation_id: Some(conversation.id),
        };
        authority::resolve(&f.state.db, &request).await.unwrap();
        for case in ["generation", "agent", "sender", "channel_generation"] {
            let mut bad = request.clone();
            let binding = bad.origin.as_mut().unwrap().thread.as_mut().unwrap();
            match case {
                "generation" => binding.generation += 1,
                "agent" => binding.agent_id = "changed".into(),
                "sender" => binding.sender_id = "stranger".into(),
                "channel_generation" => binding.channel_generation += 1,
                _ => unreachable!(),
            }
            assert!(
                authority::resolve(&f.state.db, &bad).await.is_err(),
                "{case}: authority refusal"
            );
            let result = async {
                let allowed = authority::resolve(&f.state.db, &bad).await?;
                let adapter =
                    crate::services::channel_adapters::lark::LarkFamilyAdapter::media_test_adapter(
                        &f.platform.uri(),
                        "lark",
                    );
                crate::services::channel_media_fetch::fetch(
                    &f.state.db,
                    &f.state.encryption_keys,
                    &f.state.http_client,
                    &allowed.bot,
                    &allowed.source.attachments[0],
                    &adapter,
                    1024,
                )
                .await
                .map(|_| ())
            }
            .await;
            assert!(result.is_err(), "{case}");
            assert!(
                !f.platform
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .any(|r| r.url.path().contains("/resources/")),
                "{case}: zero downloads"
            );
        }
    }
}

#[tokio::test]
async fn channel_media_gateway_follow_post_text_flags_guest_and_unaddressed_first_photo() {
    use crate::models::channel_message::StoredAttachment;
    for media_on in [false, true] {
        let f = Box::pin(Fixture::new("media_gateway_follow_posts")).await;
        if media_on {
            super::media_tests::enable_media(&f.state).await;
        }
        let provider = super::media_tests::ingestion_provider(
            &f.state,
            &f.row,
            super::media_tests::native_png(),
        )
        .await;
        for (n, text, addressed, guest) in [
            (0, "", false, false),
            (1, "Receipt\nDinner", true, false),
            (2, "", false, true),
        ] {
            let (mut body, id) = f
                .input(
                    text,
                    if addressed {
                        ThreadAddress::Mention
                    } else {
                        ThreadAddress::NotAddressed
                    },
                    false,
                    true,
                )
                .await;
            let native = body["event_context"]["activity"]["thread"]["message_id"]
                .as_str()
                .unwrap();
            let item = StoredAttachment {
                content_type: "image".into(),
                provider_ref: format!(
                    "{}/open-apis/im/v1/messages/{native}/resources/img_receipt?type=image",
                    provider.uri()
                ),
                platform_message_id: Some(native.into()),
                image_key: Some("img_receipt".into()),
                file_key: None,
                filename: None,
                mime_type: None,
                size_bytes: None,
            };
            let sender = if guest { "guest" } else { "human" };
            body["event_context"]["activity"]["actor"]["id"] = json!(sender);
            body["event_context"]["activity"]["kind"]["attachments"] =
                json!([{"kind":"image","variant":"embedded"}]);
            // CMA's authenticated post flattener has removed the own-bot mention.
            body["input"][0]["content"][0]["text"] = json!(if n == 1 {
                "legacy post @Thread helper"
            } else {
                ""
            });
            f.state.db.collection::<ChannelMessage>(MESSAGES).update_one(doc! {"_id":&id}, doc! {"$set":{
                "content_type":"unknown", "sender_platform_id":sender,"embedded_attachments":[bson::to_bson(&item).unwrap()]
            }}).await.unwrap();
            let before = provider
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| r.method == "GET")
                .count();
            let answer = f.send(&body, &id).await;
            let calls = f.calls.lock().await;
            if n == 0 || (guest && !media_on) {
                assert!(!answer.contains("Here is your answer"));
                assert_eq!(calls.len(), if n == 0 { 0 } else { 1 });
            } else {
                assert!(answer.contains("Here is your answer"), "{answer}");
                let call = calls.last().unwrap();
                let text = call["input"]
                    .as_str()
                    .unwrap_or_else(|| call["input"][0]["content"][0]["text"].as_str().unwrap());
                if guest {
                    assert!(text.contains(crate::services::assistant_channel_media::GUEST_NOTE));
                    assert_eq!(
                        provider
                            .received_requests()
                            .await
                            .unwrap()
                            .iter()
                            .filter(|r| r.method == "GET")
                            .count(),
                        before
                    );
                } else if media_on {
                    assert!(text.contains("Receipt\nDinner"));
                    assert!(!text.contains("@Thread helper"));
                    assert!(call.to_string().contains("input_image"));
                } else {
                    assert!(text.contains("legacy post @Thread helper"));
                }
            }
            drop(calls);
        }
    }
}
