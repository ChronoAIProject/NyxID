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
        row.route_id = Some(Uuid::new_v4().to_string());
        row.gateway_channel_id = Some("cmaeg1.ch.test".into());
        row.gateway_bot_id = Some("ou_bot".into());
        row.gateway_version = Some(1);
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
            .insert_one(bot)
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
        state.db.collection::<bson::Document>("channel_conversations").insert_one(doc! {"_id":&row.route_id,"user_id":OWNER,"channel_bot_id":&row.channel_bot_id,"platform":"lark","platform_conversation_id":"*","platform_conversation_type":"group","agent_api_key_id":&row.route_api_key_id,"default_agent":true,"is_active":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
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
            doc! {"$unset":{"gateway_threads":""}},
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
    chats::sync_gateway_groups(&f.state, &legacy, true)
        .await
        .unwrap();
    assert_eq!(f.gateway.received_requests().await.unwrap().len(), 1);
    f.gateway.reset().await;
    f.state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$unset":{"gateway_threads.checked_at":""}},
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
    assert!(!threads::gateway::negotiated(
        &load_channel(&f.state, OWNER, &f.row.id).await.unwrap()
    ));
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
    assert!(threads::gateway::negotiated(
        &load_channel(&f.state, OWNER, &f.row.id).await.unwrap()
    ));
}
