use super::*;
use crate::models::channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

struct Guard(String);
impl Drop for Guard {
    fn drop(&mut self) {
        thread_follow::TEST_ADAPTERS.lock().unwrap().remove(&self.0);
        crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
            .lock()
            .unwrap()
            .remove(&self.0);
    }
}

#[tokio::test]
async fn discord_2185_user_and_own_role_mentions_reply_once_with_inspectable_admission() {
    Box::pin(direct_admission()).await;
}
async fn direct_admission() {
    let (state, calls, model) = setup("discord2185_direct").await;
    let _model = tokio_util::task::AbortOnDropHandle::new(model);
    let (mut row, _) = discord_channel(&state).await;
    row.transport = "direct".into();
    row.gateway_native_activation = Some("gateway_rejected_request".into());
    row.gateway_source = Some(crate::models::nyxbot_channel::GatewaySource::Discord {
        bot_id: "100".into(),
    });
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.route_id},
            doc! {"$set":{"platform":"discord","platform_conversation_type":"group"}},
        )
        .await
        .unwrap();
    // The generic provider fixture has no relay callback or usable platform
    // token. Direct follow and reply require the same credentials as a real link.
    state.db.collection::<bson::Document>("api_keys").update_one(
        doc! {"_id":&row.route_api_key_id},
        doc! {"$set":{"callback_url":format!("https://example.test/api/v1/nyxbot/relay/{}", row.id)}},
    ).await.unwrap();
    let token = state
        .encryption_keys
        .encrypt(b"test-discord-token")
        .await
        .unwrap();
    state.db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(
        doc! {"_id":&row.channel_bot_id},
        doc! {"$set":{"bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:token}}},
    ).await.unwrap();
    state.db.collection::<bson::Document>("feature_flag_overrides").insert_one(doc! {
        "_id":Uuid::new_v4().to_string(),"org_user_id":null,"flag_key":feature_flag_service::NYXBOT_THREAD_FOLLOW_FLAG_KEY,
        "target_kind":"user","target_key":OWNER,"enabled":true,"updated_by":"test","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
    }).await.unwrap();
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
        .await
        .unwrap();
    crate::services::discord_gateway::roles::observe(
        &bot,
        &json!({"t":"GUILD_CREATE","d":{"id":"300","roles":[
            {"id":"900","managed":true,"tags":{"bot_id":"100"}},
            {"id":"901","managed":true,"tags":{"bot_id":"999"}}
        ]}}),
    );
    let platform = MockServer::start().await;
    let adapter = Arc::new(
        crate::services::channel_adapters::discord::DiscordAdapter::media_test_adapter(
            &platform.uri(),
        ),
    );
    thread_follow::TEST_ADAPTERS
        .lock()
        .unwrap()
        .insert(bot.id.clone(), adapter.clone());
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .insert(bot.id.clone(), adapter);
    let _guard = Guard(bot.id.clone());
    Mock::given(path("/channels/200"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"id":"200","type":0,"guild_id":"300"})),
        )
        .mount(&platform)
        .await;
    let counter = std::sync::atomic::AtomicU32::new(1000);
    Mock::given(method("POST"))
        .and(path("/channels/200/messages"))
        .respond_with(move |_: &wiremock::Request| {
            ResponseTemplate::new(200).set_body_json(
                json!({"id":counter.fetch_add(1,std::sync::atomic::Ordering::SeqCst).to_string()}),
            )
        })
        .expect(3)
        .mount(&platform)
        .await;
    for (n, users, roles, everyone, code, turn) in [
        (
            500,
            json!([{"id":"100"}]),
            json!([]),
            false,
            "addressed_user_mention",
            true,
        ),
        (
            501,
            json!([]),
            json!(["900"]),
            false,
            "addressed_managed_role",
            true,
        ),
        (
            502,
            json!([]),
            json!(["901"]),
            false,
            "not_addressed",
            false,
        ),
        (503, json!([]), json!([]), true, "not_addressed", false),
        (
            504,
            json!([]),
            json!(["900"]),
            false,
            "addressed_managed_role",
            true,
        ),
    ] {
        if n == 504 {
            state
                .db
                .collection::<NyxbotThread>(THREADS)
                .update_many(
                    doc! {"channel_id":&row.id,"record_scope":{"$ne":"platform_thread"}},
                    doc! {"$set":{"threads":"off"}},
                )
                .await
                .unwrap();
        }
        let id = n.to_string();
        let raw = json!({"id":&id,"type":0,"channel_id":"200","guild_id":"300","author":{"id":"7","bot":false},
            "content":"sensitive body must not enter diagnostics","mentions":users,"mention_roles":roles,"mention_everyone":everyone,
            "channel":{"id":"200","type":0,"guild_id":"300"}});
        let facts = crate::services::discord_gateway::facts::authenticated_facts(
            &bot,
            &json!({"d":raw}),
            &raw["channel"],
        )
        .unwrap();
        let inbound =
            crate::services::channel_adapters::discord::parse_gateway_message(&raw).unwrap();
        let source_id = format!("source-{id}");
        let mut source = crate::services::channel_relay_service::inbound_metadata(
            &bot.id,
            row.route_id.as_deref().unwrap(),
            OWNER,
            "discord",
            &inbound,
            &row.route_api_key_id,
            &source_id,
        );
        source.thread_context = Some(facts.clone());
        state
            .db
            .collection::<ChannelMessage>(MESSAGES)
            .insert_one(&source)
            .await
            .unwrap();
        let correlation = Uuid::new_v4().to_string();
        let body = json!({"message_id":&source_id,"correlation_id":&correlation,"platform":"discord","thread_context":facts,
            "conversation":{"id":row.route_id,"platform_id":"200","type":"group"},"sender":{"platform_id":"7"},
            "content":{"type":"text","text":inbound.text},"raw_platform_data":raw});
        let bytes = serde_json::to_vec(&body).unwrap();
        let token = crate::crypto::jwt::generate_relay_callback_token(
            &state.jwt_keys,
            &state.config,
            &correlation,
            &row.route_api_key_id,
            &source_id,
            "discord",
            &sha256_hex(&bytes),
        )
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-nyxid-callback-token", token.parse().unwrap());
        let before = calls.lock().await.len();
        let (_watch, done) = watch_relay_callback(&row.id, &source_id);
        assert_eq!(
            Box::pin(relay_callback(
                State(state.clone()),
                Path(row.id.clone()),
                headers.clone(),
                Bytes::from(bytes.clone())
            ))
            .await
            .status(),
            StatusCode::ACCEPTED
        );
        tokio::time::timeout(Duration::from_secs(30), done)
            .await
            .unwrap()
            .unwrap();
        // Same source remains claimed: no new work or send on a repeated callback.
        assert_eq!(
            Box::pin(relay_callback(
                State(state.clone()),
                Path(row.id.clone()),
                headers,
                Bytes::from(bytes)
            ))
            .await
            .status(),
            StatusCode::ACCEPTED
        );
        let saved = state
            .db
            .collection::<ChannelMessage>(MESSAGES)
            .find_one(doc! {"_id":&source_id})
            .await
            .unwrap()
            .unwrap();
        let diag = saved.nyxbot_admission.as_ref().unwrap();
        assert_eq!(diag.disposition, code, "{id}: {diag:?}");
        assert_eq!(
            calls.lock().await.len(),
            before + usize::from(turn),
            "{id}: {diag:?}"
        );
        assert_eq!(diag.turn_id.is_some(), turn, "{id}");
        if turn {
            let admitted = state
                .db
                .collection::<NyxbotEvent>(EVENTS)
                .find_one(doc! {"channel_id":&row.id,"event_id":&source_id})
                .await
                .unwrap()
                .unwrap();
            let delivery = admitted.delivery.unwrap();
            assert_eq!(delivery.transport, "direct");
            assert!(delivery.origin.native_event_id.is_none());
        }
        assert_eq!(
            state
                .db
                .collection::<ChannelMessage>(MESSAGES)
                .count_documents(doc! {"direction":"outbound","reply_to_message_id":&source_id})
                .await
                .unwrap(),
            u64::from(turn),
            "each admitted source gets exactly one relay reply: {id}",
        );
        assert_eq!(diag.bot_mentioned, Some(turn));
        if n < 504 {
            assert_eq!(
                diag.thread_follow_selection.as_deref(),
                Some(if turn { "child" } else { "quiet" })
            );
            assert_eq!(diag.resolved_thread_id.is_some(), turn);
        }
        if n == 502 {
            assert_eq!(diag.broadcast_roles, Some(1));
        }
        if n == 503 {
            assert!(diag.broadcast_everyone_or_here);
        }
        let visible = serde_json::to_value(
            crate::handlers::channel_activities::ActivityItem::from(saved),
        )
        .unwrap();
        assert_eq!(visible["nyxbot_admission"]["disposition"], code);
        let encoded = visible["nyxbot_admission"].to_string();
        for forbidden in [
            "sensitive",
            "raw_platform_data",
            "\"900\"",
            "\"901\"",
            "content",
        ] {
            assert!(!encoded.contains(forbidden), "{encoded}");
        }
    }
    assert_eq!(
        state
            .db
            .collection::<NyxbotThread>(THREADS)
            .count_documents(doc! {"channel_id":&row.id,"record_scope":"platform_thread"})
            .await
            .unwrap(),
        2
    );
    let visible = crate::handlers::channel_activities::bot_activities(
        State(state.clone()),
        crate::test_utils::test_auth_user(OWNER),
        Path(bot.id.clone()),
        axum::extract::Query(crate::handlers::channel_activities::ActivityQuery {
            kind: None,
            page: None,
            per_page: None,
        }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(visible.activities.len(), 5);
    assert!(
        visible
            .activities
            .iter()
            .all(|activity| activity.nyxbot_admission.is_some())
    );
    assert_eq!(
        load_channel(&state, OWNER, &row.id)
            .await
            .unwrap()
            .gateway_native_activation
            .as_deref(),
        Some("gateway_rejected_request")
    );
}
