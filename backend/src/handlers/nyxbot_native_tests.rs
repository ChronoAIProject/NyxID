use super::*;

fn enable_thread_follow<'a>(state: &'a AppState) -> futures::future::BoxFuture<'a, ()> {
    Box::pin(async move {
        let collection = state
            .db
            .collection::<bson::Document>("feature_flag_overrides");
        let flag = doc! {
            "_id": Uuid::new_v4().to_string(),
            "org_user_id": bson::Bson::Null,
            "flag_key": feature_flag_service::NYXBOT_THREAD_FOLLOW_FLAG_KEY,
            "target_kind": "user",
            "target_key": OWNER,
            "enabled": true,
            "updated_by": "test",
            "created_at": bson::DateTime::now(),
            "updated_at": bson::DateTime::now(),
        };
        Box::pin(async move {
            collection.insert_one(flag).await.unwrap();
        })
        .await;
    })
}
use crate::{
    models::{
        nyxbot_channel::GatewaySource,
        nyxbot_gateway_response::{COLLECTION_NAME as RESPONSES, GatewayResponse as Stored},
    },
    services::nyxbot_response_recovery as recovery,
};

pub(super) fn native_channel<'a>(
    state: &'a AppState,
) -> futures::future::BoxFuture<'a, (NyxbotChannel, String)> {
    Box::pin(native_channel_impl(state))
}

async fn native_channel_impl(state: &AppState) -> (NyxbotChannel, String) {
    let (mut row, key) = channel(state, "gateway").await;
    row.platform = "discord".into();
    row.gateway_bot_id = Some("100".into());
    row.gateway_channel_id = Some("cmaeg1.ch.test".into());
    row.gateway_source = Some(GatewaySource::Discord {
        bot_id: "100".into(),
    });
    row.gateway_native_phase = Some("ready".into());
    row.owner_sender_ids = vec!["7".into()];
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
            doc! {"$set":{"platform":"discord"}},
        )
        .await
        .unwrap();
    let token = state
        .encryption_keys
        .encrypt(b"test-discord-token")
        .await
        .unwrap();
    state.db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(doc! {"_id":&row.channel_bot_id},
        doc! {"$set":{"platform":"discord","platform_bot_id":"100","bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:token}}}).await.unwrap();
    // Retained thread evidence must resolve the real route, including its
    // callback. Production native enrollment installs this refusing receiver.
    state.db.collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME).update_one(
        doc! {"_id":&row.route_api_key_id},
        doc! {"$set":{"callback_url":format!("{}/api/v1/nyxbot/relay/{}",state.config.base_url.trim_end_matches('/'),row.id)}}
    ).await.unwrap();
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
    ensure_partition(state, &key, PARTITION).await;
    (load_channel(state, OWNER, &row.id).await.unwrap(), key)
}
pub(super) fn native_event(row: &NyxbotChannel, id: &str, text: &str) -> Value {
    let event = format!("discord:100:{id}");
    json!({"conversation":PARTITION,"input":[{"content":[{"type":"input_text","text":"rendered input is not authority"}]}],
        "metadata":{"cmaeg.channel":&sha256_hex(row.gateway_channel_id.as_deref().unwrap())[..16],"cmaeg.event":event,"cmaeg.partition":"0123456789abcdef","cmaeg.actor":"7"},
        "event_context":{"schema_version":1,"channel_id":row.gateway_channel_id,"conversation_id":PARTITION,"event_ref":"opaque-native-ref",
        "activity":{"schema_version":1,"event_id":event,"source":{"type":"discord","bot_id":"100"},"actor":{"kind":"human","id":"7"},
            "conversation":{"kind":"private","id":"200","thread_id":null},"kind":{"type":"message","text":text,"mentions_bot":false,"attachments":[]}}}})
}
fn frames(wire: &str) -> Vec<Value> {
    wire.lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

#[tokio::test]
async fn native_response_replays_after_restart_with_cursor_and_current_binding() {
    let (state, calls, server) = setup("native_response_replay").await;
    let (row, key) = native_channel(&state).await;
    let bots = state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME);
    bots.update_one(
        doc! {"_id":&row.channel_bot_id},
        doc! {"$set":{"status":"pending_webhook","webhook_registered":false}},
    )
    .await
    .unwrap();
    let request = native_event(&row, "500", "Hello");
    assert!(
        respond(&state, "invalid-key", &request, "native-unauthenticated")
            .await
            .status()
            .is_client_error()
    );
    let mut wrong = request.clone();
    wrong["event_context"]["activity"]["source"]["bot_id"] = json!("999");
    assert!(
        respond(&state, &key, &wrong, "native-mismatched")
            .await
            .status()
            .is_client_error()
    );
    bots.update_one(
        doc! {"_id":&row.channel_bot_id},
        doc! {"$set":{"platform_bot_id":"999"}},
    )
    .await
    .unwrap();
    assert!(
        respond(&state, &key, &request, "native-stale-identity")
            .await
            .status()
            .is_client_error()
    );
    assert_eq!(
        channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
            .await
            .unwrap()
            .status,
        "pending_webhook"
    );
    bots.update_one(
        doc! {"_id":&row.channel_bot_id},
        doc! {"$set":{"platform_bot_id":"100"}},
    )
    .await
    .unwrap();
    let response = respond(&state, &key, &request, "native-500").await;
    let activated = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
        .await
        .unwrap();
    assert_eq!(activated.status, "active");
    assert!(!activated.webhook_registered);
    assert_eq!(response.status(), StatusCode::OK);
    let id = response.headers()["x-cma-response-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let output = body_text(response).await;
    assert!(output.contains("Here is your answer"), "{output}");
    let all = frames(&output);
    assert_eq!(all[0]["response"]["status"], "queued");
    assert_eq!(all.last().unwrap()["type"], "response.completed");
    let saved = state
        .db
        .collection::<Stored>(RESPONSES)
        .find_one(doc! {"response_id":&id})
        .await
        .unwrap()
        .unwrap();
    assert!(saved.conversation_id.is_some());
    crate::services::security_readiness_service::ready_test_replica(&state.db).await;
    let mut lookup_headers = bearer(&key);
    lookup_headers.insert("idempotency-key", "native-500".parse().unwrap());
    let found = dispatches::get(
        State(state.clone()),
        lookup_headers,
        axum::extract::Query(dispatches::Query {
            dispatched_at: None,
        }),
    )
    .await;
    assert_eq!(found.status(), StatusCode::OK);
    let found: Value = serde_json::from_str(&body_text(found).await).unwrap();
    assert_eq!(found["state"], "finished");
    assert_eq!(found["response_id"], id);
    assert_eq!(found["provider_delivery"], "none");
    assert_eq!(found["stored_response"], true);

    assert!(!String::from_utf8_lossy(&saved.payload_ciphertext).contains("Here is your answer"));
    // No broadcast receiver, producer task or process cache is needed by GET.
    let snap =
        native_responses::snapshot(State(state.clone()), Path(id.clone()), bearer(&key)).await;
    assert_eq!(snap.status(), StatusCode::OK);
    let snap: Value = serde_json::from_str(&body_text(snap).await).unwrap();
    assert_eq!(snap["metadata"], request["metadata"]);
    let resumed = native_responses::events(
        State(state.clone()),
        Path(id.clone()),
        axum::extract::Query(native_responses::Cursor {
            starting_after: Some("1".into()),
        }),
        bearer(&key),
    )
    .await;
    assert_eq!(frames(&body_text(resumed).await), all[2..]);
    assert_eq!(
        frames(&body_text(respond(&state, &key, &request, "native-500").await).await),
        all
    );
    assert_eq!(calls.lock().await.len(), 1);
    for cursor in ["01", "-1", "18446744073709551615", "99"] {
        assert_eq!(
            native_responses::events(
                State(state.clone()),
                Path(id.clone()),
                axum::extract::Query(native_responses::Cursor {
                    starting_after: Some(cursor.into())
                }),
                bearer(&key),
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST,
            "{cursor}"
        );
    }
    let mut changed = request.clone();
    changed["event_context"]["activity"]["kind"]["text"] = json!("changed command");
    assert_eq!(
        respond(&state, &key, &changed, "native-500").await.status(),
        StatusCode::CONFLICT
    );
    let keys = state
        .db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME);
    keys.update_one(
        doc! {"_id":&row.agent_api_key_id},
        doc! {"$set":{"is_active":false}},
    )
    .await
    .unwrap();
    assert_eq!(
        native_responses::snapshot(State(state.clone()), Path(id.clone()), bearer(&key))
            .await
            .status(),
        StatusCode::UNAUTHORIZED,
    );
    assert_eq!(
        native_responses::events(
            State(state.clone()),
            Path(id.clone()),
            axum::extract::Query(native_responses::Cursor {
                starting_after: None
            }),
            bearer(&key)
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED,
    );
    keys.update_one(
        doc! {"_id":&row.agent_api_key_id},
        doc! {"$set":{"is_active":true}},
    )
    .await
    .unwrap();
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"binding_id":"bnd_changed"}},
        )
        .await
        .unwrap();
    assert_eq!(
        native_responses::snapshot(State(state.clone()), Path(id), bearer(&key))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    server.abort();
}

#[tokio::test]
async fn native_recovery_is_purged_with_history_and_provider_partition() {
    let (state, _, server) = setup("native_recovery_purge").await;
    let (row, key) = native_channel(&state).await;
    let response = respond(&state, &key, &native_event(&row, "510", "Hello"), "510").await;
    let id = response.headers()["x-cma-response-id"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(body_text(response).await.contains("response.completed"));
    let stored = recovery::load(&state.db, &row, &id).await.unwrap().unwrap();
    engine::delete(&state.db, OWNER, stored.conversation_id.as_deref().unwrap())
        .await
        .unwrap();
    assert!(
        recovery::load(&state.db, &row, &id)
            .await
            .unwrap()
            .is_none()
    );
    recovery::finish(
        &state.db,
        &state.encryption_keys,
        &stored,
        Some("must not return"),
        false,
    )
    .await
    .unwrap();
    assert_eq!(
        native_responses::snapshot(State(state.clone()), Path(id), bearer(&key))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    // Even replies without assistant history are owned by the provider partition.
    let mut media = native_event(&row, "511", "");
    media["event_context"]["activity"]["kind"]["attachments"] = json!([{"kind":"image"}]);
    let response = respond(&state, &key, &media, "511").await;
    let id = response.headers()["x-cma-response-id"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        body_text(response)
            .await
            .contains(recovery::MEDIA_UNAVAILABLE)
    );
    assert_eq!(
        delete_conversation(
            State(state.clone()),
            Path((row.binding_id.clone().unwrap(), PARTITION.into())),
            bearer(&key)
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert!(
        recovery::load(&state.db, &row, &id)
            .await
            .unwrap()
            .is_none()
    );
    server.abort();
}

#[tokio::test]
async fn native_preparation_and_invalid_sources_cannot_execute() {
    let (state, calls, server) = setup("native_preparation").await;
    let (row, key) = native_channel(&state).await;
    let request = native_event(&row, "501", "Hello");
    for phase in ["preparing", "draining"] {
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{"gateway_native_phase":phase}},
            )
            .await
            .unwrap();
        let response = respond(&state, &key, &request, "501").await;
        if phase == "preparing" {
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
            assert!(!response.headers().contains_key("x-cma-response-id"));
            let body: Value = serde_json::from_str(&body_text(response).await).unwrap();
            assert_eq!(
                body,
                json!({"error":{"type":"invalid_request_error","code":"conversation_not_ready"}})
            );
        } else {
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        }
    }
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"gateway_native_phase":"ready"}},
        )
        .await
        .unwrap();
    for (pointer, value) in [
        ("/event_context/channel_id", json!("foreign")),
        ("/event_context/activity/source/type", json!("nyxid_relay")),
        ("/event_context/activity/source/bot_id", json!("101")),
        ("/event_context/activity/actor/kind", json!("bot")),
        ("/event_context/activity/actor/id", json!("100")),
        (
            "/event_context/activity/conversation/kind",
            json!("channel"),
        ),
        (
            "/event_context/activity/conversation/thread_id",
            json!("200"),
        ),
        ("/event_context/activity/kind/type", json!("reaction")),
        ("/event_context/activity/event_id", json!("discord:101:501")),
    ] {
        let mut invalid = request.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        assert_eq!(
            respond(&state, &key, &invalid, "invalid").await.status(),
            StatusCode::BAD_REQUEST,
            "{pointer}"
        );
    }
    // A source introduced by a newer replica stays readable and fails closed.
    state
        .db
        .collection::<bson::Document>(CHANNELS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"gateway_source":{"type":"future_native_source","account":"verified"}}},
        )
        .await
        .unwrap();
    assert_eq!(
        respond(&state, &key, &request, "unknown-source")
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let unknown = load_channel(&state, OWNER, &row.id).await.unwrap();
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
        .await
        .unwrap();
    assert_eq!(
        gateway_policy(&state, &unknown, &bot, &[], GATEWAY_GROUPS_DEFAULT)["sources"],
        json!([])
    );
    assert!(calls.lock().await.is_empty());
    assert_eq!(
        state
            .db
            .collection::<Stored>(RESPONSES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    server.abort();
}

#[tokio::test]
async fn native_cross_transport_duplicate_and_refusal_have_replayable_silence() {
    let (state, calls, server) = setup("native_cutover_dedup").await;
    let (row, key) = native_channel(&state).await;
    use crate::services::coordination_service::{EventDedupClaimResult, EventDedupStore};
    let scope = serde_json::to_string(&(Some(&row.channel_bot_id), OWNER)).unwrap();
    let EventDedupClaimResult::Claimed(claim) = EventDedupStore::claim(
        &state.db,
        "discord-inbound",
        &scope,
        "502",
        Duration::from_secs(30),
    )
    .await
    .unwrap() else {
        panic!()
    };
    EventDedupStore::commit(&state.db, &claim, Duration::from_secs(86400))
        .await
        .unwrap();
    let wire =
        body_text(respond(&state, &key, &native_event(&row, "502", "Hello"), "502").await).await;
    let all = frames(&wire);
    assert_eq!(all.last().unwrap()["response"]["output"], json!([]));
    let mut guest = native_event(&row, "503", "Hello");
    guest["event_context"]["activity"]["actor"]["id"] = json!("8");
    guest["metadata"]["cmaeg.actor"] = json!("8");
    let wire = body_text(respond(&state, &key, &guest, "503").await).await;
    assert!(wire.contains(PRIVATE_REFUSAL), "{wire}");
    assert!(calls.lock().await.is_empty());
    server.abort();
}

#[tokio::test]
async fn native_unadmitted_crash_becomes_durable_failure_and_purge_wins() {
    let (state, _, server) = setup("native_recovery_crash").await;
    let (row, key) = native_channel(&state).await;
    let request = native_event(&row, "504", "/start");
    let response = respond(&state, &key, &request, "504").await;
    let id = response.headers()["x-cma-response-id"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(body_text(response).await.contains("response.completed"));
    let mut saved = recovery::load(&state.db, &row, &id).await.unwrap().unwrap();
    saved.terminal = false;
    let queued = recovery::Payload::new(
        &id,
        "nyxbot_stable",
        recovery::metadata(&request["metadata"]).unwrap(),
    );
    recovery::storage::install(&state.encryption_keys, &mut saved, &queued)
        .await
        .unwrap();
    state
        .db
        .collection::<Stored>(RESPONSES)
        .replace_one(doc! {"_id":&saved.id}, &saved)
        .await
        .unwrap();
    state.db.collection::<NyxbotEvent>(EVENTS).update_one(doc! {"_id":&saved.event_key},doc! {"$set":{"created_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::minutes(3)),"status":"running"}}).await.unwrap();
    recovery::refresh(
        &state.db,
        &state.encryption_keys,
        &saved,
        std::future::ready,
    )
    .await
    .unwrap();
    let terminal = recovery::load(&state.db, &row, &id).await.unwrap().unwrap();
    assert!(terminal.terminal);
    assert_eq!(
        recovery::payload(&state.encryption_keys, &terminal)
            .await
            .unwrap()
            .snapshot["status"],
        "failed"
    );
    assert_eq!(
        delete_conversation(
            State(state.clone()),
            Path(("bnd_chats".into(), PARTITION.into())),
            bearer(&key)
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    recovery::finish(
        &state.db,
        &state.encryption_keys,
        &saved,
        Some("must not resurrect"),
        false,
    )
    .await
    .unwrap();
    assert!(
        recovery::load(&state.db, &row, &id)
            .await
            .unwrap()
            .is_none()
    );
    server.abort();
}

#[tokio::test]
async fn native_source_policy_credentials_and_legacy_golden_payloads() {
    let (state, _, server) = setup("native_policy_goldens").await;
    let (mut row, _) = native_channel(&state).await;
    let mut bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
        .await
        .unwrap();
    for platform in ["telegram", "lark", "feishu"] {
        row.platform = platform.into();
        row.gateway_source = None;
        row.gateway_bot_id = Some("ou_bot".into());
        bot.platform_bot_username = "helper_bot".into();
        let actual = gateway_policy(
            &state,
            &row,
            &bot,
            &["route-1".into()],
            GATEWAY_GROUPS_DEFAULT,
        );
        let mut source = json!({"type":"nyxid_relay","issuer":state.config.jwt_issuer,"key_id":row.route_api_key_id,
            "route_ids":["route-1"],"platform":platform,"admission":{"type":"scoped","senders":{"type":"open"},"chats":{"type":"open"},"groups":"mention_or_reply_to_bot"}});
        if platform == "telegram" {
            source["bot_username"] = json!("helper_bot");
        } else {
            source["bot_id"] = json!("ou_bot");
        }
        let name = format!("NyxBot {}", row.bot_label);
        let expected = json!({"schema_version":1,"name":name,"profile":{"schema_version":1,
            "metadata":{"name":name,"description":"NyxBot, the owner's NyxID assistant","owner":{"subject":row.user_id,"kind":"human"},"provider":"nyxbot"},
            "model":{"access":"managed","service":engine::SERVICE_SLUG,"api_type":"responses","model":engine::DEFAULT_MODEL,"thinking_effort":"medium"},
            "permission":{"action_mode":"autonomous","service_scope":[]},"context":{"system_prompt":"","skills":[],"mcp_servers":[]}},
            "provider":{"slug":"nyxbot"},"sources":[source],"partition":{"version":1,"key":"conversation_and_sender"},
            "reply":{"auto_final":true,"ack_after_ms":0,"sinks":[{"type":"source"}]}});
        assert_eq!(actual.to_string(), expected.to_string(), "{platform}");
        assert_eq!(
            crate::services::nyxbot_gateway_source::credentials(
                &state, &row, &bot, "agent", "route"
            )
            .await
            .unwrap(),
            json!({"agent_key":"agent","channel_key":"route"})
        );
    }
    row.platform = "discord".into();
    row.gateway_bot_id = Some("100".into());
    row.gateway_source = Some(GatewaySource::Discord {
        bot_id: "100".into(),
    });
    let native = gateway_policy(
        &state,
        &row,
        &bot,
        &["route-1".into()],
        GATEWAY_GROUPS_DEFAULT,
    );
    assert_eq!(
        native["sources"],
        json!([{"type":"discord","bot_id":"100","intents":37377,
        "admission":{"type":"scoped","senders":{"type":"open"},"chats":{"type":"open"},"groups":"all"}}])
    );
    let credentials =
        crate::services::nyxbot_gateway_source::credentials(&state, &row, &bot, "agent", "route")
            .await
            .unwrap();
    assert_eq!(
        credentials,
        json!({"agent_key":"agent","discord_bot_token":"test-discord-token"})
    );
    assert!(native["sources"][0].get("key_id").is_none());
    assert_eq!(
        native["routing"]["addressing"],
        json!({"mentions":false,"command":false,"event_field":false,"unknown_target_notice":false})
    );
    assert!(native["routing"].get("consumers").is_none());
    assert!(!gateway_bot_mentions::desired(&row, &bot));
    row.gateway_source = None;
    assert_eq!(
        gateway_policy(&state, &row, &bot, &[], GATEWAY_GROUPS_DEFAULT)["sources"][0]["trust_normalized"],
        true
    );
    server.abort();
}

pub(super) struct NativeDiscordFixtureGuard(pub(super) String);
impl Drop for NativeDiscordFixtureGuard {
    fn drop(&mut self) {
        crate::services::nyxbot_gateway_source::TEST_DISCORD_BASES
            .lock()
            .unwrap()
            .remove(&self.0);
        thread_follow::TEST_ADAPTERS.lock().unwrap().remove(&self.0);
    }
}
#[tokio::test]
async fn native_guild_messages_share_history_but_retain_independent_reply_sources() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
    let (state, calls, server) =
        setup_with_reply("native_guild_identity", GATEWAY_LINK_REPLY).await;
    let (row, key) = native_channel(&state).await;
    let http = MockServer::start().await;
    crate::services::nyxbot_gateway_source::TEST_DISCORD_BASES
        .lock()
        .unwrap()
        .insert(row.channel_bot_id.clone(), http.uri());
    let _guard = NativeDiscordFixtureGuard(row.channel_bot_id.clone());
    Mock::given(path("/channels/200"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"id":"200","type":0,"guild_id":"300"})),
        )
        .mount(&http)
        .await;
    let mut saved = Vec::new();
    for id in ["610", "611"] {
        Mock::given(path(format!("/channels/200/messages/{id}"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id":id,"channel_id":"200","type":0,"author":{"id":"7"},"content":"Hello",
            "mentions":[{"id":"100"}],"mention_roles":[],"mention_everyone":false,"attachments":[],
        }))).mount(&http).await;
        let mut request = native_event(&row, id, "Hello");
        request["event_context"]["activity"]["conversation"]["kind"] = json!("group");
        request["event_context"]["activity"]["kind"]["mentions_bot"] = json!(true);
        let response = respond(&state, &key, &request, id).await;
        let response_id = response.headers()["x-cma-response-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let wire = body_text(response).await;
        assert_gateway_link_frames(&frames(&wire));
        saved.push(
            recovery::load(&state.db, &row, &response_id)
                .await
                .unwrap()
                .unwrap(),
        );
    }
    assert_eq!(calls.lock().await.len(), 2);
    assert_eq!(saved[0].conversation_id, saved[1].conversation_id);
    assert_ne!(saved[0].event_key, saved[1].event_key);
    let snapshot = native_responses::snapshot(
        State(state.clone()),
        Path(saved[0].response_id.clone()),
        bearer(&key),
    )
    .await;
    assert_eq!(snapshot.status(), StatusCode::OK);
    let snapshot: Value = serde_json::from_str(&body_text(snapshot).await).unwrap();
    assert_eq!(
        snapshot["output"][0]["content"][0]["text"],
        GATEWAY_LINK_REPLY
    );
    server.abort();
}

#[tokio::test]
async fn native_discord_authenticated_reply_evidence_is_bound_and_fail_closed() {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, path},
    };
    let (state, _, upstream) = setup("native_discord_evidence").await;
    let (row, _) = native_channel(&state).await;
    let http = MockServer::start().await;
    crate::services::nyxbot_gateway_source::TEST_DISCORD_BASES
        .lock()
        .unwrap()
        .insert(row.channel_bot_id.clone(), http.uri());
    let _guard = NativeDiscordFixtureGuard(row.channel_bot_id.clone());
    let mut request = native_event(&row, "600", "Hello");
    request["event_context"]["activity"]["conversation"] =
        json!({"kind":"group","id":"201","thread_id":"201"});
    let a = &request["event_context"]["activity"];
    let raw = json!({"id":"600","channel_id":"201","type":19,"author":{"id":"7","bot":false},
        "content":"Hello","mentions":[],"mention_roles":[],"mention_everyone":false,"attachments":[],
        "message_reference":{"message_id":"900","channel_id":"201"}});
    Mock::given(path("/channels/201/messages/600"))
        .and(header("authorization", "Bot test-discord-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(raw))
        .mount(&http)
        .await;
    Mock::given(path("/channels/201"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":"201","type":11,"parent_id":"200","guild_id":"300"})),
        )
        .mount(&http)
        .await;
    // An unverified referenced author supplies no positive reply evidence.
    let result = native_discord::evidence(&state, &row, a, "none")
        .await
        .unwrap();
    assert!(!result.addressed);
    Mock::given(path("/channels/201/messages/900"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":"900","channel_id":"201","author":{"id":"100"}})),
        )
        .mount(&http)
        .await;
    let result = native_discord::evidence(&state, &row, a, "none")
        .await
        .unwrap();
    assert!(result.addressed);
    let mut forged = a.clone();
    forged["actor"]["id"] = json!("8");
    forged["kind"]["mentions_bot"] = json!(true);
    assert!(
        !native_discord::evidence(&state, &row, &forged, "none")
            .await
            .unwrap()
            .addressed
    );
    assert!(
        http.received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method.as_str() == "GET")
    );
    upstream.abort();
}

#[tokio::test]
async fn native_discord_follow_reuses_members_expiry_stop_and_other_mentions() {
    Box::pin(native_discord_follow_reuses_members_expiry_stop_and_other_mentions_impl()).await;
}

async fn native_discord_follow_reuses_members_expiry_stop_and_other_mentions_impl() {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{path, path_regex},
    };
    let (state, calls, upstream) = setup("native_discord_follow").await;
    let (row, key) = native_channel(&state).await;
    enable_thread_follow(&state).await;
    let http = Box::pin(MockServer::start()).await;
    crate::services::nyxbot_gateway_source::TEST_DISCORD_BASES
        .lock()
        .unwrap()
        .insert(row.channel_bot_id.clone(), http.uri());
    thread_follow::TEST_ADAPTERS.lock().unwrap().insert(
        row.channel_bot_id.clone(),
        Arc::new(
            crate::services::channel_adapters::discord::DiscordAdapter::media_test_adapter(
                &http.uri(),
            ),
        ),
    );
    let _guard = NativeDiscordFixtureGuard(row.channel_bot_id.clone());
    Mock::given(path("/channels/201")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"201","type":11,"parent_id":"200","guild_id":"300","thread_metadata":{"archived":false,"locked":false}}))).mount(&http).await;
    Mock::given(path("/channels/201/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&http)
        .await;
    Mock::given(path("/guilds/300/roles"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id":"900","managed":true,"tags":{"bot_id":"100"}},
            {"id":"901","managed":true,"tags":{"bot_id":"999"}}
        ])))
        .expect(1)
        .mount(&http)
        .await;
    Mock::given(path_regex("^/channels/201/messages/[0-9]+$")).respond_with(|r:&wiremock::Request| {
        let id=r.url.path().rsplit('/').next().unwrap();
        let mention=match id {"601"=>json!([{"id":"100"}]),"603"=>json!([{"id":"8"}]),_=>json!([])};
        ResponseTemplate::new(200).set_body_json(json!({"id":id,"channel_id":"201","type":0,
            "author":{"id":"7","bot":false},"content":if id=="604" {"stop following"} else {"Hello"},
            "mentions":mention,"mention_roles":if id=="607" {json!(["900"])} else {json!([])},"mention_everyone":false,"attachments":[]}))
    }).mount(&http).await;
    for (id, expect_turn) in [
        ("601", true),
        ("602", true),
        ("603", false),
        ("606", false),
        ("604", false),
        ("605", false),
        ("607", true),
    ] {
        if id == "606" {
            state.db.collection::<NyxbotThread>(THREADS).update_many(doc! {"channel_id":&row.id,"record_scope":"platform_thread"},
                doc! {"$set":{"follow_expires_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::seconds(1))}}).await.unwrap();
        }
        if id == "604" {
            state.db.collection::<NyxbotThread>(THREADS).update_many(doc! {"channel_id":&row.id,"record_scope":"platform_thread"},
                doc! {"$set":{"follow_state":"active","follow_expires_at":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::hours(1))}}).await.unwrap();
        }
        let mut request = native_event(
            &row,
            id,
            if id == "604" {
                "stop following"
            } else {
                "Hello"
            },
        );
        request["event_context"]["activity"]["conversation"] =
            json!({"kind":"group","id":"201","thread_id":"201"});
        request["event_context"]["activity"]["kind"]["mentions_bot"] = json!(id == "601");
        let before = calls.lock().await.len();
        let output = body_text(respond(&state, &key, &request, id).await).await;
        assert!(output.contains("response.completed"), "{id}: {output}");
        assert_eq!(
            calls.lock().await.len(),
            before + usize::from(expect_turn),
            "{id}: {output}"
        );
        if id == "607" {
            let source = state
                .db
                .collection::<crate::models::channel_message::ChannelMessage>(
                    crate::models::channel_message::COLLECTION_NAME,
                )
                .find_one(doc! {"channel_bot_id":&row.channel_bot_id,"platform_message_id":id})
                .await
                .unwrap()
                .unwrap();
            let diagnostic = source.nyxbot_admission.unwrap();
            assert_eq!(diagnostic.disposition, "addressed_managed_role");
            assert!(diagnostic.turn_id.is_some());
            assert_eq!(diagnostic.thread_follow_selection.as_deref(), Some("child"));
        }
        if id == "601" {
            assert_eq!(state.db.collection::<NyxbotThread>(THREADS).count_documents(
                doc! {"channel_id":&row.id,"record_scope":"platform_thread","follow_state":"active"}
            ).await.unwrap(),1,"the mention must activate following, not just the ordinary guild path");
        }
    }
    let child = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"channel_id":&row.id,"record_scope":"platform_thread"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(child.follow.follow_state.as_deref(), Some("active"));
    assert!(
        http.received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method.as_str() == "GET"),
        "CMA alone owns source sends"
    );
    upstream.abort();
}

pub(super) async fn native_flag(state: &AppState, enabled: bool) {
    state.db.collection::<bson::Document>("feature_flag_overrides").insert_one(doc! {
        "_id":Uuid::new_v4().to_string(),"org_user_id":bson::Bson::Null,
        "flag_key":"nyxbot:gateway-discord","target_kind":"user","target_key":OWNER,"enabled":enabled,
        "updated_by":"test","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
    }).await.unwrap();
}

#[tokio::test]
async fn activated_native_gateway_select_serves_overlap_and_rejects_unsupported() {
    use crate::services::{
        nyxbot_gateway_source as source, security_readiness_service as readiness,
    };
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
    let (mut state, _, upstream) = setup("native_roster_overlap").await;
    let (row, _) = native_channel(&state).await;
    let (_, gateway) = mock_gateway(
        &mut state,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
    .await;
    let http = MockServer::start().await;
    source::TEST_DISCORD_BASES
        .lock()
        .unwrap()
        .insert(row.channel_bot_id.clone(), http.uri());
    let _guard = NativeDiscordFixtureGuard(row.channel_bot_id.clone());
    Mock::given(path("/users/@me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"100","bot":true})))
        .expect(1)
        .mount(&http)
        .await;
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
        .await
        .unwrap();
    readiness::ready_test_replica(&state.db).await;
    let sibling = readiness::overlapping_test_replica(&state.db).await;
    assert!(matches!(
        source::select(&state, &row, &bot).await.unwrap(),
        Some(GatewaySource::Discord { .. })
    ));
    readiness::remove_test_replica_capability(&state.db, &sibling, source::READINESS).await;
    assert!(matches!(
        source::select(&state, &row, &bot).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    assert_eq!(http.received_requests().await.unwrap().len(), 1);
    gateway.abort();
    upstream.abort();
}

#[tokio::test]
async fn native_enrollment_both_paths_gate_preparation_and_preserve_source_on_reconcile() {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, path},
    };
    for fresh in [false, true] {
        let (mut state, _, upstream) = setup("native_enrollment").await;
        let (mut row, _) = native_channel(&state).await;
        let (calls, gateway) = mock_gateway(
            &mut state,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .await;
        native_flag(&state, true).await;
        let http = MockServer::start().await;
        crate::services::nyxbot_gateway_source::TEST_DISCORD_BASES
            .lock()
            .unwrap()
            .insert(row.channel_bot_id.clone(), http.uri());
        let _guard = NativeDiscordFixtureGuard(row.channel_bot_id.clone());
        Mock::given(path("/users/@me"))
            .and(header("authorization", "Bot test-discord-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"100","bot":true})))
            .mount(&http)
            .await;
        let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
            .await
            .unwrap();
        // No mixed-fleet claim is inferred from a local binary's capabilities.
        assert!(
            crate::services::nyxbot_gateway_source::select(&state, &row, &bot)
                .await
                .is_err()
        );
        assert!(calls.lock().await.is_empty());
        crate::services::security_readiness_service::ready_test_replica(&state.db).await;
        if fresh {
            state
                .db
                .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
                .delete_one(doc! {"_id":&row.route_id})
                .await
                .unwrap();
            state
                .db
                .collection::<NyxbotChannel>(CHANNELS)
                .delete_one(doc! {"_id":&row.id})
                .await
                .unwrap();
            let agent = crate::services::assistant_team_service::ensure_nyxbot(&state.db, OWNER)
                .await
                .unwrap();
            row = connect(&state, OWNER, None, &bot.id, &agent)
                .await
                .unwrap()
                .0;
        } else {
            state.db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {"_id":&row.id},
                doc! {"$set":{"transport":"direct"},"$unset":{"gateway_source":"","gateway_native_phase":"","binding_id":"","gateway_channel_id":""}}).await.unwrap();
            row = load_channel(&state, OWNER, &row.id).await.unwrap();
            move_to_gateway(&state, &row).await.unwrap();
        }
        row = load_channel(&state, OWNER, &row.id).await.unwrap();
        assert!(crate::services::nyxbot_gateway_source::ready(&row));
        if fresh {
            let audit = state.db.collection::<crate::models::audit_log::AuditLog>(crate::models::audit_log::COLLECTION_NAME)
                .find_one(doc! {"event_type":"nyxbot_channel_connected","event_data.channel_agent_id":&row.id}).await.unwrap().unwrap().event_data.unwrap();
            assert_eq!(audit["transport"], "gateway");
            assert_eq!(audit["gateway_native_activation"], "pending_discovery");
        }
        assert_eq!(row.gateway_bot_id.as_deref(), Some("100"));
        assert_eq!(
            row.gateway_native_activation.as_deref(),
            Some("pending_discovery")
        );
        assert!(
            !crate::services::discord_gateway::transport::relay_allowed(&state.db, &bot.id)
                .await
                .unwrap()
        );
        assert_eq!(
            chats::sync_gateway_groups(&state, &row, true)
                .await
                .unwrap(),
            None
        );
        let writes = calls.lock().await;
        for (_, _, body) in writes
            .iter()
            .filter(|c| (c.0 == "POST" || c.0 == "PUT") && !c.1.ends_with("/routing"))
        {
            assert!(body.get("routing").is_none());
            assert_eq!(body["sources"].as_array().unwrap().len(), 1);
            assert_eq!(body["sources"][0]["type"], "discord");
            assert_eq!(body["sources"][0]["bot_id"], "100");
            assert!(body["credentials"].get("channel_key").is_none());
            assert_eq!(body["sources"][0]["admission"]["groups"], "all");
        }
        drop(writes);
        let key = key_service::get_api_key(&state.db, OWNER, &row.route_api_key_id)
            .await
            .unwrap();
        assert!(
            key.callback_url
                .as_deref()
                .unwrap()
                .contains("/api/v1/nyxbot/relay/")
        );
        state
            .db
            .collection::<bson::Document>("feature_flag_overrides")
            .update_many(
                doc! {"flag_key":"nyxbot:gateway-discord","target_key":OWNER},
                doc! {"$set":{"enabled":false}},
            )
            .await
            .unwrap();
        switch_to_gateway(&state).await.unwrap();
        let returned = load_channel(&state, OWNER, &row.id).await.unwrap();
        assert_eq!(returned.transport, "direct");
        assert!(returned.gateway_source.is_none());
        assert_eq!(
            returned.gateway_native_activation.as_deref(),
            Some("relay_draining")
        );
        assert_eq!(returned.route_id, row.route_id);
        assert!(
            !crate::services::discord_gateway::transport::relay_allowed(&state.db, &bot.id)
                .await
                .unwrap(),
            "DELETE does not join CMA's socket worker"
        );
        assert!(calls.lock().await.iter().any(|c| c.0 == "DELETE"));
        assert!(
            crate::services::nyxbot_gateway_source::select(&state, &returned, &bot)
                .await
                .is_err(),
            "native re-enrollment also waits for the previous socket drain"
        );
        gateway.abort();
        upstream.abort();
    }
}

#[tokio::test]
async fn native_queued_response_recovers_without_reposting_after_observers_disconnect() {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    let (state, _, upstream) = setup("native_queued_recovery").await;
    let (row, key) = native_channel(&state).await;
    let http = MockServer::start().await;
    let completed = json!({"type":"response.completed","sequence_number":1,"response":{
        "id":"resp_11111111111111111111111111111111_22222222222222222222222222222222","conversation":{"id":"conv_11111111111111111111111111111111"},"status":"completed",
        "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Queued answer"}]}]}});
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(move |_: &wiremock::Request| {
            let mut completed = completed.clone();
            completed["response"]["id"] = json!(format!(
                "resp_11111111111111111111111111111111_{}",
                Uuid::new_v4().simple()
            ));
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(5))
                .set_body_raw(format!("data: {completed}\n\n"), "text/event-stream")
        })
        .mount(&http)
        .await;
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .update_one(
            doc! {"slug":engine::SERVICE_SLUG},
            doc! {"$set":{"base_url":http.uri()}},
        )
        .await
        .unwrap();
    let first = respond(
        &state,
        &key,
        &native_event(&row, "701", "First question"),
        "701",
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first_id = first.headers()["x-cma-response-id"]
        .to_str()
        .unwrap()
        .to_owned();
    drop(first);
    wait_native_bound(&state, &row, &first_id, false).await;
    let second = respond(
        &state,
        &key,
        &native_event(&row, "702", "Second question"),
        "702",
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    let id = second.headers()["x-cma-response-id"]
        .to_str()
        .unwrap()
        .to_owned();
    drop(second);
    let stored = wait_native_bound(&state, &row, &id, true).await;
    let event = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&stored.event_key})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.status, "queued");
    assert!(event.turn_id.is_none());
    assert!(stored.conversation_id.is_some());
    let deletion = deletion(&row, "702", "7");
    let response = respond(&state, &key, &deletion, "delete-queued").await;
    assert_eq!(response.status(), StatusCode::OK);
    let notification = frames(&body_text(response).await);
    assert_eq!(
        notification.last().unwrap()["response"]["output"],
        json!([])
    );
    recovery::refresh(
        &state.db,
        &state.encryption_keys,
        &stored,
        std::future::ready,
    )
    .await
    .unwrap();
    let replay = native_responses::events(
        State(state.clone()),
        Path(id),
        axum::extract::Query(native_responses::Cursor {
            starting_after: None,
        }),
        bearer(&key),
    )
    .await;
    let wire = body_text(replay).await;
    if !wire.contains("Queued answer") {
        let errors: Vec<bson::Document> = state
            .db
            .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
            .find(doc! {"conversation_id":&stored.conversation_id,"role":"assistant"})
            .projection(
                doc! {"_id":0,"status":1,"error_code":1,"upstream_error_code":1,"error_detail":1},
            )
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        panic!("queued recovery failed: {errors:?}\n{wire}");
    }
    assert_eq!(frames(&wire).last().unwrap()["type"], "response.completed");
    assert_eq!(
        http.received_requests().await.unwrap().len(),
        2,
        "GET recovery never POSTs a turn"
    );
    upstream.abort();
}

async fn wait_native_bound(
    state: &AppState,
    row: &NyxbotChannel,
    id: &str,
    queued: bool,
) -> Stored {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let stored = recovery::load(&state.db, row, id).await.unwrap().unwrap();
            let event = state
                .db
                .collection::<NyxbotEvent>(EVENTS)
                .find_one(doc! {"_id":&stored.event_key})
                .await
                .unwrap()
                .unwrap();
            if stored.conversation_id.is_some()
                && (if queued {
                    event.status == "queued"
                } else {
                    event.turn_id.is_some()
                })
            {
                return stored;
            }
            assert!(!stored.terminal, "dispatch failed before turn admission");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("native dispatch did not bind")
}

#[tokio::test]
async fn native_acceptance_precedes_slow_platform_reads_and_survives_observer_drop() {
    use futures::StreamExt;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
    let (state, calls, upstream) = setup("native_acceptance_budget").await;
    let (row, key) = native_channel(&state).await;
    let http = MockServer::start().await;
    crate::services::nyxbot_gateway_source::TEST_DISCORD_BASES
        .lock()
        .unwrap()
        .insert(row.channel_bot_id.clone(), http.uri());
    let _guard = NativeDiscordFixtureGuard(row.channel_bot_id.clone());
    Mock::given(path("/channels/200/messages/790")).respond_with(
        ResponseTemplate::new(200).set_delay(Duration::from_secs(5)).set_body_json(json!({
            "id":"790","channel_id":"200","type":0,"author":{"id":"7","bot":false},
            "content":"Hello","mentions":[{"id":"100"}],"mention_everyone":false,"mention_roles":[],"attachments":[]
        }))
    ).mount(&http).await;
    Mock::given(path("/channels/200"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"id":"200","type":0,"guild_id":"300"})),
        )
        .mount(&http)
        .await;
    let mut request = native_event(&row, "790", "Hello");
    request["event_context"]["activity"]["conversation"]["kind"] = json!("group");
    request["event_context"]["activity"]["kind"]["mentions_bot"] = json!(true);
    let id = tokio::time::timeout(Duration::from_secs(2), async {
        let accepted = respond(&state, &key, &request, "790").await;
        assert_eq!(accepted.status(), StatusCode::OK);
        let id = accepted.headers()["x-cma-response-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let mut stream = accepted.into_body().into_data_stream();
        let first = stream.next().await.unwrap().unwrap();
        assert!(String::from_utf8_lossy(&first).contains("id: 0\nevent: response.created"));
        // CMA drops its POST observer after the durable acceptance prefix.
        id
    })
    .await
    .expect("acceptance waited for Discord evidence");
    let replay = native_responses::events(
        State(state.clone()),
        Path(id),
        axum::extract::Query(native_responses::Cursor {
            starting_after: Some("0".into()),
        }),
        bearer(&key),
    )
    .await;
    assert!(body_text(replay).await.contains("Here is your answer"));
    assert_eq!(calls.lock().await.len(), 1);
    upstream.abort();
}

#[tokio::test]
async fn native_media_is_explicit_and_async_updates_keep_original_event() {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    let (state, calls, upstream) = setup("native_media_and_updates").await;
    let (row, key) = native_channel(&state).await;
    let mut request = native_event(&row, "703", "");
    request["event_context"]["activity"]["kind"]["attachments"] = json!([{"kind":"image","reference":"discord:703:1","source":{"type":"discord","channel_id":"200","message_id":"703","attachment_id":"1"}}]);
    let output = body_text(respond(&state, &key, &request, "703").await).await;
    assert!(output.contains(recovery::MEDIA_UNAVAILABLE), "{output}");
    assert!(calls.lock().await.is_empty());
    let media_event = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"channel_id":&row.id,"event_id":"discord:100:703"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(media_event.source_media_declared, Some(1));
    assert!(
        media_event.media.is_none(),
        "native media never acquires a fetch claim"
    );
    let response = respond(&state, &key, &native_event(&row, "704", "Hello"), "704").await;
    let id = response.headers()["x-cma-response-id"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(body_text(response).await.contains("Here is your answer"));
    let stored = recovery::load(&state.db, &row, &id).await.unwrap().unwrap();
    let event = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&stored.event_key})
        .await
        .unwrap()
        .unwrap();
    let origin = event.delivery.as_ref().unwrap().origin.clone();
    let http = MockServer::start().await;
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.slug = GATEWAY_SLUG.into();
    service.base_url = http.uri();
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .insert_one(service)
        .await
        .unwrap();
    Mock::given(method("POST"))
        .and(path("/events/opaque-native-ref/replies"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"delivery":{"outcome":"confirmed"}})),
        )
        .mount(&http)
        .await;
    let newer = state
        .encryption_keys
        .encrypt(b"newer-reference-must-not-be-used")
        .await
        .unwrap();
    state.db.collection::<NyxbotThread>(THREADS).update_many(doc! {"channel_id":&row.id},doc! {"$set":{"event_ref_ciphertext":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:newer}}}).await.unwrap();
    let mut invalid = deletion(&row, "704", "7");
    invalid["metadata"]["cmaeg.event"] = json!("mismatch");
    assert_eq!(
        respond(&state, &key, &invalid, "invalid-deletion")
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert!(intake_refusal_recommended(&state, &row).await);
    native_responses::send_update(
        &state,
        &row,
        &origin,
        stored.conversation_id.as_deref().unwrap(),
        "Update",
    )
    .await
    .unwrap();
    assert_eq!(http.received_requests().await.unwrap().len(), 1);
    assert_eq!(
        load_channel(&state, OWNER, &row.id)
            .await
            .unwrap()
            .gateway_native_delivery
            .as_deref(),
        Some("reply_confirmed")
    );
    assert!(!intake_refusal_recommended(&state, &row).await);
    let chat = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"channel_id":&row.id,"partition":&origin.partition})
        .await
        .unwrap()
        .unwrap();
    let plan=images::ImageSendPlan { delegated: Vec::new(),attachments:vec![serde_json::from_value(json!({"id":"tool-image","content_type":"image/png","size":10,"label":"image","origin":"tool","chat_delivery":true})).unwrap()]};
    for anchor in [
        images::ImageAnchor::Inbound {
            message_id: "stale-relay".into(),
        },
        images::ImageAnchor::Thread {
            origin: origin.clone(),
        },
        images::ImageAnchor::Initiated {
            chat_id: chat.id.clone(),
        },
    ] {
        let result = images::send_turn_images(
            &state,
            images::ImageSendContext {
                source: crate::models::image_preparation::Source::Event {
                    event_id: String::new(),
                },
                owner: OWNER,
                conversation_id: stored.conversation_id.as_deref().unwrap(),
                turn_id: "turn",
                channel: &row,
                thread: &chat,
                anchor,
                audience: images::ImageAudience {
                    owner_private: true,
                },
                deadline: None,
                initiated: None,
                fence: None,
            },
            &plan,
        )
        .await;
        assert!(matches!(result, Err(AppError::ChannelMediaUnsupported)));
    }
    assert_eq!(
        http.received_requests().await.unwrap().len(),
        1,
        "image anchors cause no native or relay send"
    );
    let mut unbound = origin;
    unbound.native_event_id = None;
    assert!(
        native_responses::send_update(
            &state,
            &row,
            &unbound,
            stored.conversation_id.as_deref().unwrap(),
            "Refused"
        )
        .await
        .is_err()
    );
    upstream.abort();
}

#[tokio::test]
async fn native_async_reply_thread_mode_and_delivery_receipts_are_source_bound() {
    Box::pin(native_async_reply_thread_mode_and_delivery_receipts_are_source_bound_impl()).await;
}

async fn native_async_reply_thread_mode_and_delivery_receipts_are_source_bound_impl() {
    for native_thread in [false, true] {
        Box::pin(native_async_reply_thread_mode_and_delivery_receipts_iteration(native_thread))
            .await;
    }
}

async fn native_async_reply_thread_mode_and_delivery_receipts_iteration(native_thread: bool) {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    let (state, _, upstream) = setup("native_async_thread_receipts").await;
    let (row, key) = native_channel(&state).await;
    enable_thread_follow(&state).await;
    let discord = Box::pin(MockServer::start()).await;
    crate::services::nyxbot_gateway_source::TEST_DISCORD_BASES
        .lock()
        .unwrap()
        .insert(row.channel_bot_id.clone(), discord.uri());
    thread_follow::TEST_ADAPTERS.lock().unwrap().insert(
        row.channel_bot_id.clone(),
        Arc::new(
            crate::services::channel_adapters::discord::DiscordAdapter::media_test_adapter(
                &discord.uri(),
            ),
        ),
    );
    let _guard = NativeDiscordFixtureGuard(row.channel_bot_id.clone());
    let channel = if native_thread {
        json!({"id":"201","type":11,"parent_id":"200","guild_id":"300","thread_metadata":{"archived":false,"locked":false}})
    } else {
        json!({"id":"201","type":0,"guild_id":"300"})
    };
    Mock::given(path("/channels/201"))
        .respond_with(ResponseTemplate::new(200).set_body_json(channel))
        .mount(&discord)
        .await;
    Mock::given(path("/channels/201/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&discord)
        .await;
    Mock::given(path("/channels/201/messages/801")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id":"801","channel_id":"201","type":0,"author":{"id":"7","bot":false},
            "content":"Hello","mentions":[{"id":"100"}],"mention_roles":[],"mention_everyone":false,"attachments":[],
        }))).mount(&discord).await;
    let mut request = native_event(&row, "801", "Hello");
    request["event_context"]["activity"]["conversation"] =
        json!({"kind":"group","id":"201","thread_id":if native_thread {Some("201")} else {None}});
    request["event_context"]["activity"]["kind"]["mentions_bot"] = json!(true);
    let response = respond(&state, &key, &request, "801").await;
    let id = response.headers()["x-cma-response-id"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(body_text(response).await.contains("Here is your answer"));
    let stored = recovery::load(&state.db, &row, &id).await.unwrap().unwrap();
    let event = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":&stored.event_key})
        .await
        .unwrap()
        .unwrap();
    let origin = &event.delivery.as_ref().unwrap().origin;
    assert!(
        origin.thread.is_some(),
        "both native threads and local reply chains are followed"
    );
    let gateway = Box::pin(MockServer::start()).await;
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.slug = GATEWAY_SLUG.into();
    service.base_url = gateway.uri();
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .insert_one(service)
        .await
        .unwrap();
    for occurrence in [None, Some("permission-result-occurrence")] {
        for (receipt, expected) in [
            (
                json!({"delivery":{"outcome":"confirmed"}}),
                "reply_confirmed",
            ),
            (
                json!({"delivery":{"outcome":"rejected","reason":"test"}}),
                "reply_rejected",
            ),
            (
                json!({"delivery":{"outcome":"unsupported"}}),
                "reply_unsupported",
            ),
            (
                json!({"delivery":{"outcome":"rate_limited","retry_after":1}}),
                "reply_rate_limited",
            ),
            (
                json!({"delivery":{"outcome":"unknown","reason":"test"}}),
                "reply_unknown",
            ),
            (json!({}), "reply_unknown"),
        ] {
            gateway.reset().await;
            Mock::given(method("POST"))
                .and(path("/events/opaque-native-ref/replies"))
                .respond_with(ResponseTemplate::new(200).set_body_json(receipt))
                .expect(1)
                .mount(&gateway)
                .await;
            // A later ambient turn cannot take ownership of this source's delivery.
            let other_turn = crate::turn_trace::TurnContext::for_turn(&Uuid::new_v4().to_string());
            let (result, trace) =
                crate::turn_trace::capture(crate::turn_trace::scope(other_turn, async {
                    if let Some(occurrence) = occurrence {
                        native_responses::send_occurrence(
                            &state,
                            &row,
                            origin,
                            stored.conversation_id.as_deref().unwrap(),
                            "Update",
                            occurrence,
                        )
                        .await
                    } else {
                        native_responses::send_update(
                            &state,
                            &row,
                            origin,
                            stored.conversation_id.as_deref().unwrap(),
                            "Update",
                        )
                        .await
                        .map(|()| "confirmed")
                    }
                }))
                .await;
            let expected_outcome = match expected {
                "reply_confirmed" => "confirmed",
                "reply_rejected" | "reply_unsupported" | "reply_rate_limited" => "rejected",
                _ => "unknown",
            };
            if occurrence.is_some() {
                assert_eq!(result.unwrap(), expected_outcome, "{expected}");
            } else {
                assert_eq!(result.is_ok(), expected == "reply_confirmed", "{expected}");
            }
            let original_turn = event.turn_id.as_deref().unwrap();
            assert!(!trace.is_empty());
            assert!(
                trace
                    .iter()
                    .all(|record| record["turn_id"] == original_turn)
            );
            let sent = trace
                .iter()
                .find(|record| record["stage"] == "channel" && record["phase"] == "end")
                .unwrap();
            assert_eq!(sent["outcome"], expected_outcome);
            let part_end = trace
                .iter()
                .find(|record| record["stage"] == "channel_part" && record["phase"] == "end")
                .unwrap();
            assert_eq!(part_end["outcome"], expected_outcome);
            let part = trace
                .iter()
                .find(|record| record["stage"] == "channel_part" && record["phase"] == "start")
                .unwrap();
            let gateway_span = trace
                .iter()
                .find(|record| record["stage"] == "gateway" && record["phase"] == "start")
                .unwrap();
            assert_eq!(gateway_span["parent_span_id"], part["span_id"]);
            let requests = gateway.received_requests().await.unwrap();
            assert_eq!(
                requests.len(),
                1,
                "no automatic retry of an uncertain receipt"
            );
            assert_eq!(
                requests[0]
                    .headers
                    .get("idempotency-key")
                    .map(|value| value.to_str().unwrap()),
                occurrence.map(|_| "permission-result-occurrence:0"),
            );
            let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
            assert_eq!(
                body.get("thread_reply"),
                native_thread.then_some(&Value::Bool(true))
            );
            let saved = load_channel(&state, OWNER, &row.id).await.unwrap();
            assert_eq!(saved.gateway_native_delivery.as_deref(), Some(expected));
        }
    }
    upstream.abort();
}

#[tokio::test]
async fn native_gateway_problem_code_is_bounded_persisted_surfaced_and_attempt_fenced() {
    let (state, _, server) = setup("native_problem_code").await;
    let (row, _) = native_channel(&state).await;
    let channels = state.db.collection::<NyxbotChannel>(CHANNELS);
    channels
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"gateway_setup_release_id":"attempt"}},
        )
        .await
        .unwrap();
    let response = GatewayResponse {
        status: 400,
        body: json!({"code":"discord.invalid_intents-v1", "detail":"private token https://private.example"}),
    };
    record_gateway_rejection(&state, &row, "attempt", &response, "create")
        .await
        .unwrap();
    let saved = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(
        saved.gateway_native_activation.as_deref(),
        Some("gateway_rejected_request")
    );
    assert_eq!(
        saved.gateway_native_activation_detail.as_deref(),
        Some("discord.invalid_intents-v1")
    );
    let api = serde_json::to_value(ChannelAgentResponse::from(&saved)).unwrap();
    assert_eq!(
        api["gateway_native_activation_detail"],
        "discord.invalid_intents-v1"
    );
    assert!(!api.to_string().contains("private token"));
    assert_eq!(
        gateway_diagnostic_audit(json!({"channel_agent_id":row.id}), &saved)["gateway_native_activation_detail"],
        "discord.invalid_intents-v1"
    );
    // A late response from an old preparation cannot replace these diagnostics.
    record_gateway_rejection(
        &state,
        &row,
        "older-attempt",
        &GatewayResponse {
            status: 400,
            body: json!({"code":"stale"}),
        },
        "channel_update",
    )
    .await
    .unwrap();
    assert_eq!(
        load_channel(&state, OWNER, &row.id)
            .await
            .unwrap()
            .gateway_native_activation_detail,
        saved.gateway_native_activation_detail
    );
    crate::services::nyxbot_gateway_source::record_activation(
        &state.db,
        &row,
        &json!({"native_discord_activation":{"state":"selected"}}),
    )
    .await
    .unwrap();
    assert!(
        load_channel(&state, OWNER, &row.id)
            .await
            .unwrap()
            .gateway_native_activation_detail
            .is_none()
    );
    for code in [
        Value::Null,
        json!(""),
        json!("Uppercase"),
        json!("human prose"),
        json!("https://private.example"),
        json!("a".repeat(65)),
        json!("private\ncode"),
        json!(17),
    ] {
        record_gateway_rejection(
            &state,
            &row,
            "attempt",
            &GatewayResponse {
                status: 400,
                body: json!({"code":code,"detail":"private token"}),
            },
            "create",
        )
        .await
        .unwrap();
        let saved = load_channel(&state, OWNER, &row.id).await.unwrap();
        assert!(saved.gateway_native_activation_detail.is_none());
        let api = serde_json::to_value(ChannelAgentResponse::from(&saved)).unwrap();
        assert!(api.get("gateway_native_activation_detail").is_none());
        assert!(!api.to_string().contains("private token"));
    }
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn discord_intake_evidence_requires_authenticated_current_binding_and_refreshes_active_bots()
{
    use crate::services::channel_bot_health::discord;
    let (state, _, server) = setup("discord_native_intake_evidence").await;
    let (row, key) = native_channel(&state).await;
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
        .await
        .unwrap();
    let before = discord::load(&state.db, &[bot]).await.unwrap();
    assert_eq!(
        before[&row.channel_bot_id].code,
        Some(discord::NATIVE_UNCONFIRMED)
    );
    let mut invalid = native_event(&row, "511", "Hello");
    invalid["event_context"]["activity"]["source"]["bot_id"] = json!("999");
    assert!(
        respond(&state, &key, &invalid, "invalid-evidence")
            .await
            .status()
            .is_client_error()
    );
    assert!(
        load_channel(&state, OWNER, &row.id)
            .await
            .unwrap()
            .gateway_intake
            .is_none()
    );
    let response = respond(
        &state,
        &key,
        &native_event(&row, "512", "Hello"),
        "valid-evidence",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = body_text(response).await;
    let after = load_channel(&state, OWNER, &row.id).await.unwrap();
    let evidence = after.gateway_intake.as_ref().unwrap();
    assert_eq!(Some(&evidence.binding_id), after.binding_id.as_ref());
    assert!(evidence.last_inbound_at.is_some());
    assert!(
        evidence.handed_over_at.is_none(),
        "legacy hand-over time is unknown"
    );
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
        .await
        .unwrap();
    assert_eq!(
        bot.diagnostics.last_inbound_source.as_deref(),
        Some("native_discord_gateway")
    );
    assert!(bot.diagnostics.last_inbound_at.is_some());
    assert!(
        discord::load(&state.db, std::slice::from_ref(&bot))
            .await
            .unwrap()[&bot.id]
            .code
            .is_none()
    );
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"binding_id":"new-binding"}},
        )
        .await
        .unwrap();
    assert_eq!(
        discord::load(&state.db, &[bot]).await.unwrap()[&row.channel_bot_id].code,
        Some(discord::NATIVE_UNCONFIRMED)
    );
    let listing = channel_listing(&state, OWNER, false).await.unwrap();
    assert_eq!(
        listing["channel_agents"][0]["delivery_status"],
        "unconfirmed"
    );
    assert_eq!(
        listing["channel_agents"][0]["discord_intake"]["receiving_side"],
        "native_gateway"
    );
    server.abort();
}

// A message already admitted on relay still proves authenticated native intake,
// but produces no downstream turn. This observes the admission transaction
// without racing a later successful provider response/delivery update.
async fn confirmed_duplicate_intake(state: &AppState, row: &NyxbotChannel, key: &str, id: &str) {
    use crate::services::coordination_service::{EventDedupClaimResult, EventDedupStore};
    let scope = serde_json::to_string(&(Some(&row.channel_bot_id), bot_owner(row))).unwrap();
    let EventDedupClaimResult::Claimed(claim) = EventDedupStore::claim(
        &state.db,
        "discord-inbound",
        &scope,
        id,
        Duration::from_secs(30),
    )
    .await
    .unwrap() else {
        panic!("unique test message");
    };
    EventDedupStore::commit(&state.db, &claim, Duration::from_secs(86400))
        .await
        .unwrap();
    let response = respond(state, key, &native_event(row, id, "Hello"), id).await;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = body_text(response).await;
}

#[tokio::test]
async fn native_intake_admission_clears_only_uncertainty_and_throttles_live_binding_evidence() {
    let (state, calls, server) = setup("native_intake_throttling").await;
    let (row, key) = native_channel(&state).await;
    let rows = state.db.collection::<NyxbotChannel>(CHANNELS);
    rows.update_one(
        doc! {"_id":&row.id},
        doc! {"$set":{
            "delivery_status":"unconfirmed","delivery_error":"discord_native_intake_unconfirmed",
        }},
    )
    .await
    .unwrap();
    confirmed_duplicate_intake(&state, &row, &key, "600").await;
    let first = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(first.delivery_status.as_deref(), Some("unknown"));
    assert!(first.delivery_error.is_none());
    let first_at = first.gateway_intake.unwrap().last_inbound_at.unwrap();
    confirmed_duplicate_intake(&state, &row, &key, "601").await;
    let second = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(
        second.gateway_intake.unwrap().last_inbound_at,
        Some(first_at),
        "recent evidence does not write again"
    );
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
        .await
        .unwrap();
    assert!(bot.diagnostics.last_inbound_at.unwrap() >= first_at);
    assert_eq!(
        bot.diagnostics.last_inbound_source.as_deref(),
        Some("native_discord_gateway")
    );

    // A stale health writer's code is cleared even when evidence is fresh;
    // clearing it uses the existing admission fence, not a forced evidence write.
    rows.update_one(
        doc! {"_id":&row.id},
        doc! {"$set":{
            "delivery_status":"unconfirmed","delivery_error":"discord_native_intake_unconfirmed",
        }},
    )
    .await
    .unwrap();
    let listed = channel_listing(&state, OWNER, false).await.unwrap();
    assert_eq!(listed["channel_agents"][0]["delivery_status"], "unknown");
    assert!(listed["channel_agents"][0]["delivery_error"].is_null());
    confirmed_duplicate_intake(&state, &row, &key, "602").await;
    let cleared = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert_eq!(cleared.delivery_status.as_deref(), Some("unknown"));
    assert!(cleared.delivery_error.is_none());
    assert_eq!(
        cleared.gateway_intake.unwrap().last_inbound_at,
        Some(first_at)
    );

    let old = Utc::now() - ChronoDuration::seconds(61);
    rows.update_one(
        doc! {"_id":&row.id},
        doc! {"$set":{"gateway_intake.last_inbound_at":bson::DateTime::from_chrono(old)}},
    )
    .await
    .unwrap();
    confirmed_duplicate_intake(&state, &row, &key, "603").await;
    let refreshed = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert!(refreshed.gateway_intake.unwrap().last_inbound_at.unwrap() > old);

    rows.update_one(
        doc! {"_id":&row.id},
        doc! {"$set":{"gateway_intake.binding_id":"previous-binding"}},
    )
    .await
    .unwrap();
    confirmed_duplicate_intake(&state, &row, &key, "604").await;
    let rebound = load_channel(&state, OWNER, &row.id)
        .await
        .unwrap()
        .gateway_intake
        .unwrap();
    assert_eq!(Some(&rebound.binding_id), row.binding_id.as_ref());
    assert!(rebound.handed_over_at.is_none());

    let handover = bson::DateTime::from_chrono(Utc::now() - ChronoDuration::seconds(10));
    rows.update_one(doc!{"_id":&row.id},doc!{"$set":{
        "gateway_intake.handed_over_at":handover,
        "gateway_intake.last_inbound_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::seconds(30)),
    }}).await.unwrap();
    confirmed_duplicate_intake(&state, &row, &key, "605").await;
    let moved = load_channel(&state, OWNER, &row.id)
        .await
        .unwrap()
        .gateway_intake
        .unwrap();
    assert_eq!(moved.handed_over_at, Some(handover.to_chrono()));
    assert!(moved.last_inbound_at.unwrap() >= handover.to_chrono());

    for (id, status, error) in [
        ("606", "failing", "refused_401"),
        ("607", "unconfirmed", "real_delivery_error"),
    ] {
        rows.update_one(doc!{"_id":&row.id},doc!{"$set":{"delivery_status":status,"delivery_error":error},"$unset":{"gateway_intake":""}}).await.unwrap();
        confirmed_duplicate_intake(&state, &row, &key, id).await;
        let failure = load_channel(&state, OWNER, &row.id).await.unwrap();
        assert_eq!(failure.delivery_status.as_deref(), Some(status));
        assert_eq!(failure.delivery_error.as_deref(), Some(error));
        assert!(failure.gateway_intake.unwrap().last_inbound_at.is_some());
        let listed = channel_listing(&state, OWNER, false).await.unwrap();
        assert_eq!(listed["channel_agents"][0]["delivery_error"], error);
    }
    assert!(calls.lock().await.is_empty());
    server.abort();
}

#[tokio::test]
async fn supplemental_native_progress_leaves_primary_replay_intact() {
    Box::pin(async {
        let (state, _, _) = setup("supplemental_native_progress").await;
        let (channel, key) = native_channel(&state).await;
        let request = native_event(&channel, "500", "Hello");
        let response = Box::pin(respond(&state, &key, &request, "native-supplemental")).await;
        let id = response.headers()["x-cma-response-id"].to_str().unwrap().to_owned();
        assert!(body_text(response).await.contains("Here is your answer"));
        let stored = state.db.collection::<Stored>(RESPONSES).find_one(doc!{"response_id":&id}).await.unwrap().unwrap();
        let conversation = engine::get(&state.db, OWNER, stored.conversation_id.as_deref().unwrap()).await.unwrap();
        let event = state.db.collection::<NyxbotEvent>(EVENTS).find_one(doc!{"_id":&stored.event_key}).await.unwrap().unwrap();
        let origin = &event.delivery.as_ref().unwrap().origin;
        let baseline = body_text(native_responses::snapshot(State(state.clone()), Path(id.clone()), bearer(&key)).await).await;
        let gateway = wiremock::MockServer::start().await;
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = Uuid::new_v4().to_string();
        service.slug = GATEWAY_SLUG.into();
        service.base_url = gateway.uri();
        state.db.collection::<DownstreamService>(SERVICES).insert_one(service).await.unwrap();
        for purpose in [supplemental::Purpose::CheckingNotice, supplemental::Purpose::CapacityNotice, supplemental::Purpose::MachineDesktopNotice] {
            deliver_to_for(&state, &conversation, origin, "progress", None, purpose).await;
        }
        assert!(gateway.received_requests().await.unwrap().is_empty());
        let after = body_text(native_responses::snapshot(State(state.clone()), Path(id), bearer(&key)).await).await;
        assert_eq!(baseline, after);
        let event = state.db.collection::<NyxbotEvent>(EVENTS).find_one(doc!{"_id":&stored.event_key}).await.unwrap().unwrap();
        assert_eq!(event.delivery.unwrap().state, "replay");
        let audits: Vec<bson::Document> = state.db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME).find(doc!{"event_type":"nyxbot_channel_supplemental_delivery"}).await.unwrap().try_collect().await.unwrap();
        assert_eq!(audits.len(), 3);
        for audit in audits { assert_eq!(audit.get_document("event_data").unwrap().get_str("reason"), Ok("native_progress_unsupported")); }
        wiremock::Mock::given(wiremock::matchers::path("/events/opaque-native-ref/replies"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({"delivery":{"outcome":"confirmed"}})))
            .expect(2).mount(&gateway).await;
        deliver_to(&state,&conversation,origin,"independent result",None).await;
        let turn = Uuid::new_v4().to_string();
        for _ in 0..2 {
            assert_eq!(native_responses::send_supplemental(
                &state, &channel, origin, &conversation.id, "saved native result", Some(&turn),
            ).await.unwrap(), "confirmed");
        }
        let saved_key = crate::services::channel_supplemental_delivery::saved_key(&turn, origin);
        let plan = crate::services::channel_text_delivery::load(&state.db, &saved_key).await.unwrap().unwrap();
        assert_eq!(plan.channel_bot_id.as_deref(), Some(channel.channel_bot_id.as_str()));
        assert_eq!(plan.user_id, channel.user_id);
        assert_eq!(plan.source_id, event.id);
        gateway.verify().await;
        let audit=state.db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME).find_one(doc!{"event_type":"nyxbot_channel_supplemental_delivery","event_data.purpose":"team_result"}).await.unwrap().unwrap();
        assert_eq!(audit.get_document("event_data").unwrap().get_str("egress"),Ok("gateway_event"));
        state.db.drop().await.unwrap();
    }).await;
}

/// A Discord deletion uses the original partition even though Discord omits the deleting actor.
pub(super) fn deletion(row: &NyxbotChannel, message: &str, author: &str) -> Value {
    let mut request = native_event(row, message, "rendered input");
    let original = format!("discord:100:{message}");
    let id = format!("{original}:delete");
    let activity = &mut request["event_context"]["activity"];
    activity["event_id"] = json!(id);
    activity["occurred_at"] = json!("2026-11-19T03:23:17Z");
    activity["actor"] = json!({"kind":"unknown","id":"unknown"});
    activity["kind"] = json!({"type":"event","ce":{"specversion":"1.0","type":"cma.message.deleted.v1",
        "id":id,"source":"urn:cmaeg:discord:100","subject":message,"datacontenttype":"application/json",
        "data":{"message_id":message,"original_event_id":original,"original_author":{"kind":"human","id":author}}}});
    request["stream"] = json!(true);
    request["input"][0]["role"] = json!("user");
    request["metadata"]["cmaeg.event"] = json!(id);
    request["metadata"]["cmaeg.actor"] = json!("unknown");
    request
}

#[tokio::test]
async fn native_discord_notification_completes_and_replays_without_a_turn() {
    let (state, calls, server) = setup("native_discord_notification").await;
    let (row, key) = native_channel(&state).await;
    let original = respond(
        &state,
        &key,
        &native_event(&row, "501", "First question"),
        "first",
    )
    .await;
    assert!(body_text(original).await.contains("Here is your answer"));
    let command = deletion(&row, "501", "7");
    crate::services::security_readiness_service::ready_test_replica(&state.db).await;
    let lookup = || async {
        let mut headers = bearer(&key);
        headers.insert("idempotency-key", "delete-command".parse().unwrap());
        let response = dispatches::get(
            State(state.clone()),
            headers,
            axum::extract::Query(dispatches::Query {
                dispatched_at: None,
            }),
        )
        .await;
        serde_json::from_str::<Value>(&body_text(response).await).unwrap()
    };
    assert_eq!(lookup().await["state"], "not_received");
    let response = respond(&state, &key, &command, "delete-command").await;
    assert_eq!(response.status(), StatusCode::OK);
    let id = response.headers()["x-cma-response-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let all = frames(&body_text(response).await);
    assert_eq!(all[0]["sequence_number"], 0);
    assert_eq!(all[0]["response"]["status"], "queued");
    let found = lookup().await;
    assert_eq!(found["state"], "finished");
    assert_eq!(found["outcome"], "completed");
    assert_eq!(found["response_id"], id);
    assert_eq!(found["provider_delivery"], "none");
    let complete = all.last().unwrap();
    assert_eq!(complete["type"], "response.completed");
    assert_eq!(complete["response"]["output"], json!([]));
    assert_eq!(complete["response"]["metadata"], command["metadata"]);
    let snapshot =
        native_responses::snapshot(State(state.clone()), Path(id.clone()), bearer(&key)).await;
    assert_eq!(
        serde_json::from_str::<Value>(&body_text(snapshot).await).unwrap(),
        complete["response"]
    );
    let replay = native_responses::events(
        State(state.clone()),
        Path(id),
        axum::extract::Query(native_responses::Cursor {
            starting_after: Some("0".into()),
        }),
        bearer(&key),
    )
    .await;
    assert_eq!(frames(&body_text(replay).await), all[1..]);
    assert_eq!(
        frames(&body_text(respond(&state, &key, &command, "delete-command").await).await),
        all
    );
    assert_eq!(calls.lock().await.len(), 1);
    let event = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"event_id":"discord:100:501:delete"})
        .await
        .unwrap()
        .unwrap();
    assert!(event.turn_id.is_none() && event.conversation_id.is_none());
    let next = respond(
        &state,
        &key,
        &native_event(&row, "503", "Next question"),
        "next",
    )
    .await;
    assert!(body_text(next).await.contains("Here is your answer"));
    assert_eq!(calls.lock().await.len(), 2);
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn native_discord_notification_edits_reactions_and_controls_are_silent() {
    let (state, calls, server) = setup("discord_non_message").await;
    let (row, key) = native_channel(&state).await;
    for (revision, event_type, actor) in [
        (
            "501:edit:1795060000000000000",
            "cma.message.updated.v1",
            "human",
        ),
        (
            "reaction:ac040955-3609-4dcb-8310-6a1ccfe1799e",
            "cma.message.reaction.updated.v1",
            "unknown",
        ),
        (
            "reaction:ac040955-3609-4dcb-8310-6a1ccfe1799f",
            "cma.message.reaction.updated.v1",
            "human",
        ),
        ("interaction:508", "interaction", "human"),
    ] {
        let mut command = deletion(&row, "501", "7");
        let id = format!("discord:100:{revision}");
        let activity = &mut command["event_context"]["activity"];
        activity["event_id"] = json!(id);
        if actor == "human" {
            activity["actor"] = json!({"kind":"human","id":"7"});
        }
        if event_type == "interaction" {
            activity["kind"] =
                json!({"type":"interaction","action_id":"confirm","block_id":"test","values":{}});
        } else {
            let ce = &mut activity["kind"]["ce"];
            ce["id"] = json!(id);
            ce["type"] = json!(event_type);
            if event_type == "cma.message.updated.v1" {
                ce["data"]["patch"] = json!({"text":"Edited text"});
            } else {
                ce["data"]["operation"] = json!(if actor == "human" { "add" } else { "clear_all" });
                ce["data"]["target_kind"] = json!("inbound");
                ce["data"]["target_author"] = json!({"kind":"human","id":"7"});
                ce["data"]["representation"] = json!("delta");
                if actor == "human" {
                    ce["data"]["emoji"] = json!({"id":null,"name":"👍"});
                    ce["data"]["reaction_type"] = json!(0);
                    ce["data"]["burst"] = json!(false);
                }
            }
        }
        command["metadata"]["cmaeg.event"] = json!(id);
        command["metadata"]["cmaeg.actor"] = json!(if actor == "human" { "7" } else { "unknown" });
        if event_type == "cma.message.reaction.updated.v1" {
            let mut changed = command.clone();
            let data = &mut changed["event_context"]["activity"]["kind"]["ce"]["data"];
            data["original_event_id"] = json!("discord:100:499");
            assert_eq!(
                respond(&state, &key, &changed, revision).await.status(),
                StatusCode::BAD_REQUEST
            );
            if actor == "human" {
                let data = &mut changed["event_context"]["activity"]["kind"]["ce"]["data"];
                data["target_kind"] = json!("outbound");
                assert_eq!(
                    respond(&state, &key, &changed, revision).await.status(),
                    StatusCode::BAD_REQUEST
                );
                changed["event_context"]["activity"]["kind"]["ce"]["data"]["target_author"] =
                    json!({"kind":"bot","id":"100"});
                let mut other_bot = changed.clone();
                other_bot["event_context"]["activity"]["kind"]["ce"]["data"]["original_event_id"] =
                    json!("discord:101:499");
                assert_eq!(
                    respond(&state, &key, &other_bot, revision).await.status(),
                    StatusCode::BAD_REQUEST
                );
                command = changed;
            }
        }
        let response = respond(&state, &key, &command, revision).await;
        assert_eq!(response.status(), StatusCode::OK, "{revision}");
        let all = frames(&body_text(response).await);
        assert_eq!(all.len(), 3);
        assert_eq!(all[2]["type"], "response.completed");
        assert_eq!(all[2]["response"]["output"], json!([]));
    }
    assert!(calls.lock().await.is_empty());
    server.abort();
    state.db.drop().await.unwrap();
}

async fn intake_refusal_recommended(state: &AppState, row: &NyxbotChannel) -> bool {
    crate::services::nyxbot_gateway_release_service::ensure_indexes(&state.db)
        .await
        .unwrap();
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
        .await
        .unwrap();
    crate::handlers::channel_bots::health_loaded(state, &bot, true, true)
        .await
        .unwrap()
        .recommended_action_text
        .contains("nyxbot_provider_refused")
}

#[tokio::test]
async fn native_discord_notification_checks_source_before_acceptance_and_reports_refusal() {
    let (state, calls, server) = setup("discord_notification_health").await;
    let (row, key) = native_channel(&state).await;
    let command = deletion(&row, "501", "8");
    for (pointer, value) in [
        ("/event_context/activity/source/bot_id", json!("101")),
        (
            "/event_context/activity/kind/ce/source",
            json!("urn:cmaeg:discord:101"),
        ),
        ("/event_context/activity/kind/ce/subject", json!("502")),
        (
            "/event_context/activity/kind/ce/type",
            json!("future.unknown.v1"),
        ),
        (
            "/event_context/activity/kind/ce/id",
            json!("discord:100:502:delete"),
        ),
        (
            "/event_context/activity/actor",
            json!({"kind":"human","id":"8"}),
        ),
        ("/metadata/cmaeg.actor", json!("8")),
    ] {
        let mut invalid = command.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        assert_eq!(
            respond(&state, &key, &invalid, "refusal").await.status(),
            StatusCode::BAD_REQUEST,
            "{pointer}"
        );
    }
    assert!(intake_refusal_recommended(&state, &row).await);
    let channels = state.db.collection::<NyxbotChannel>(CHANNELS);
    for (status, phase) in [("paused", "ready"), ("active", "preparing")] {
        channels
            .update_one(
                doc! {"_id":&row.id},
                doc! {"$set":{"status":status,"gateway_native_phase":phase}},
            )
            .await
            .unwrap();
        assert!(!intake_refusal_recommended(&state, &row).await);
    }
    channels
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"status":"active","gateway_native_phase":"ready"}},
        )
        .await
        .unwrap();
    assert!(intake_refusal_recommended(&state, &row).await);
    let good = respond(&state, &key, &command, "refusal").await;
    assert_eq!(good.status(), StatusCode::OK);
    assert_eq!(
        load_channel(&state, OWNER, &row.id)
            .await
            .unwrap()
            .gateway_native_delivery
            .as_deref(),
        Some("response_ready")
    );
    assert!(!intake_refusal_recommended(&state, &row).await);
    assert!(calls.lock().await.is_empty());
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn native_discord_notification_health_ignores_legacy_and_concurrent_message_refusals() {
    use crate::services::coordination_service::{EventDedupClaimResult, EventDedupStore};
    let (state, calls, server) = setup("discord_admission_conflict_health").await;
    let (row, key) = native_channel(&state).await;
    let channels = state.db.collection::<NyxbotChannel>(CHANNELS);
    channels
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"gateway_native_delivery":"intake_refused"}},
        )
        .await
        .unwrap();
    assert!(!intake_refusal_recommended(&state, &row).await);
    let scope = serde_json::to_string(&(Some(&row.channel_bot_id), OWNER)).unwrap();
    assert!(matches!(
        EventDedupStore::claim(
            &state.db,
            "discord-inbound",
            &scope,
            "501",
            Duration::from_secs(30)
        )
        .await
        .unwrap(),
        EventDedupClaimResult::Claimed(_)
    ));
    let conflict = respond(
        &state,
        &key,
        &native_event(&row, "501", "Hello"),
        "duplicate",
    )
    .await;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    assert!(!intake_refusal_recommended(&state, &row).await);
    assert!(calls.lock().await.is_empty());
    server.abort();
    state.db.drop().await.unwrap();
}

pub(super) async fn native_whatsapp_channel(state: &AppState) -> (NyxbotChannel, String) {
    use crate::models::channel_whatsapp_native as wa;
    let (mut row, key) = native_channel(state).await;
    row.platform = "whatsapp".into();
    row.gateway_source = Some(GatewaySource::WhatsApp {
        app_id: "111".into(),
        waba_id: "222".into(),
        phone_number_id: "333".into(),
    });
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    let bots = state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME);
    bots.update_one(doc!{"_id":&row.channel_bot_id},doc!{"$set":{"platform":"whatsapp","platform_bot_id":"333","app_id":"222","app_secret_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![1]}}}).await.unwrap();
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
        .await
        .unwrap();
    let configs = state.db.collection::<bson::Document>(wa::COLLECTION_NAME);
    configs.insert_one(doc!{"_id":&bot.id,"owner_id":&bot.user_id,"ownership_version":bot.ownership_version,
        "meta_app_id":"111","waba_id":"222","phone_number_id":"333","dedicated_app":true,"updated_at":bson::DateTime::now(),
        "callback":{"phase":"gateway","gateway_channel_id":&row.gateway_channel_id,"gateway_verified_at":bson::DateTime::now()}}).await.unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &row.route_id},
            doc! {"$set": {"platform": "whatsapp"}},
        )
        .await
        .unwrap();
    (row, key)
}

#[tokio::test]
async fn native_whatsapp_notifications_keep_live_custody_and_complete_without_a_turn() {
    let (state, calls, server) = setup("whatsapp_notifications").await;
    let (row, key) = native_whatsapp_channel(&state).await;
    for kind in [
        "cma.message.updated.v1",
        "cma.message.deleted.v1",
        "cma.message.reaction.updated.v1",
        "interaction",
        "flow",
    ] {
        let prefix = if kind == "flow" {
            "native-whatsapp-flow"
        } else {
            "whatsapp"
        };
        let id = format!("{prefix}:{}", sha256_hex(kind));
        let mut request = whatsapp_request(&row, &id, "155501", "");
        let a = &mut request["event_context"]["activity"];
        a["kind"] = if matches!(kind, "interaction" | "flow") {
            json!({"type":"interaction","block_id":"card","action_id":"button","values":{}})
        } else {
            json!({"type":"event","ce":{"specversion":"1.0","id":id,"type":kind,"source":"urn:cma:whatsapp:111:222:333",
                "subject":"wamid.original","data":{"message_id":"wamid.original","original_event_id":"original"}}})
        };
        let response = respond(&state, &key, &request, kind).await;
        assert_eq!(response.status(), StatusCode::OK, "{kind}");
        let all = frames(&body_text(response).await);
        assert_eq!(all.len(), 3);
        assert_eq!(all[2]["type"], "response.completed");
        assert_eq!(all[2]["response"]["output"], json!([]));
        if kind == "flow" {
            state
                .db
                .collection::<bson::Document>(
                    crate::models::channel_whatsapp_native::COLLECTION_NAME,
                )
                .update_one(
                    doc! {"_id":&row.channel_bot_id},
                    doc! {"$set":{"callback.phase":"relay"}},
                )
                .await
                .unwrap();
            assert_eq!(
                respond(&state, &key, &request, "new-command")
                    .await
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
    }
    assert!(calls.lock().await.is_empty());
    server.abort();
    state.db.drop().await.unwrap();
}

pub(super) fn whatsapp_request(row: &NyxbotChannel, id: &str, sender: &str, text: &str) -> Value {
    let mut request = native_event(row, id, text);
    request["metadata"]["cmaeg.event"] = json!(id);
    request["metadata"]["cmaeg.actor"] = json!(sender);
    let activity = &mut request["event_context"]["activity"];
    activity["event_id"] = json!(id);
    activity["source"] =
        json!({"type":"whatsapp","app_id":"111","waba_id":"222","phone_number_id":"333"});
    activity["actor"] = json!({"kind":"unknown","id":sender});
    activity["conversation"] = json!({"kind":"private","id":sender});
    request
}

#[tokio::test]
async fn whatsapp_private_turn_reuse_and_queued_wake_after_restart() {
    Box::pin(whatsapp_private_turns()).await;
}

async fn whatsapp_private_turns() {
    let (state, calls, server) = setup("whatsapp_private_turns").await;
    let (row, key) = native_whatsapp_channel(&state).await;
    let events = state.db.collection::<NyxbotEvent>(EVENTS);
    let mut conversation = None;
    let mut turns = std::collections::HashSet::new();
    for (id, text) in [
        ("wamid.first", "First question"),
        ("wamid.second", "Next question"),
    ] {
        let response = respond(&state, &key, &whatsapp_request(&row, id, "7", text), id).await;
        assert_eq!(response.status(), StatusCode::OK);
        let wire = body_text(response).await;
        assert_eq!(
            frames(&wire).last().unwrap()["response"]["output"][0]["content"][0]["text"],
            "Here is your answer"
        );
        let event = events
            .find_one(doc! {"channel_id":&row.id,"event_id":id})
            .await
            .unwrap()
            .unwrap();
        let chat = engine::get(&state.db, OWNER, event.conversation_id.as_deref().unwrap())
            .await
            .unwrap();
        assert!(chat.active_turn.is_none());
        assert!(turns.insert(event.turn_id.unwrap()));
        assert_eq!(conversation.get_or_insert(chat.id.clone()), &chat.id);
    }
    let id = conversation.unwrap();
    let prior = engine::begin_turn(
        &state.db,
        OWNER,
        &engine::TurnRequest {
            conversation_id: Some(id.clone()),
            text: "An existing turn".into(),
            agent_id: None,
            model: None,
            access_mode: None,
            retry_of: None,
            acknowledgement_id: None,
            attachment_ids: Vec::new(),
        },
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let full: Vec<_> = (0..engine::MAX_PENDING_EVENTS)
        .map(|_| {
            crate::services::assistant_team_service::event("system", "Pending update".into(), None)
        })
        .collect();
    engine::push_events(&state.db, OWNER, &id, full)
        .await
        .unwrap();
    let chat = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"channel_id":&row.id, "platform_chat_id":"7"})
        .await
        .unwrap()
        .unwrap();
    for (event_id, language, expected) in [
        (
            "wamid.refused",
            None,
            "NyxBot is busy. Please try again shortly.",
        ),
        (
            "wamid.refused.zh",
            Some("zh-CN"),
            "NyxBot 正忙，请稍后再试。",
        ),
    ] {
        if let Some(language) = language {
            chats::update_chat(
                &state,
                OWNER,
                &chat.id,
                &chats::ChatSettings {
                    notice_language: Some(language.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        }
        let refused = whatsapp_request(&row, event_id, "7", "When will there be capacity?");
        let refusal = respond(&state, &key, &refused, event_id).await;
        assert_eq!(refusal.status(), StatusCode::OK);
        let refused_frames = frames(&body_text(refusal).await);
        assert_eq!(
            refused_frames.last().unwrap()["response"]["output"][0]["content"][0]["text"],
            expected
        );
    }
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
        .await
        .unwrap();
    let health = crate::handlers::channel_bots::health_for(&bot, true, true);
    assert_eq!(
        health.diagnostics.last_inbound_source.as_deref(),
        Some("native_whatsapp_gateway")
    );
    assert_eq!(
        health.diagnostics.last_inbound_outcome.unwrap().code,
        "whatsapp_turn_busy"
    );
    assert_eq!(
        engine::get(&state.db, OWNER, &id)
            .await
            .unwrap()
            .pending_events
            .len(),
        engine::MAX_PENDING_EVENTS
    );
    crate::services::assistant_team_service::drop_events(&state.db, OWNER, &id)
        .await
        .unwrap();
    let input = whatsapp_request(&row, "wamid.queued", "7", "A distinct queued question");
    let response = respond(&state, &key, &input, "wa-queued").await;
    let response_id = response.headers()["x-cma-response-id"]
        .to_str()
        .unwrap()
        .to_owned();
    drop(response);
    let stored = wait_native_bound(&state, &row, &response_id, true).await;
    let pending = engine::get(&state.db, OWNER, &id).await.unwrap();
    assert!(
        pending
            .pending_events
            .iter()
            .any(|e| e.source_event_ids.contains(&stored.event_key))
    );

    // The predecessor stopped after committing settlement but before its wake.
    // Resume with fresh in-memory workers over the same durable queue.
    engine::finish_turn(
        &state.db,
        &prior,
        &prior.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: "Existing turn finished".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    let restarted = test_app_state(state.db.clone());
    let work = crate::services::assistant_team_service::queued(&restarted.db, None)
        .await
        .unwrap();
    let queued = work
        .iter()
        .find(|chat| chat.id == id)
        .expect("restart sweep must discover the chat");
    Box::pin(crate::handlers::assistant_team::wake(
        &restarted,
        &queued.user_id,
        &queued.id,
    ))
    .await;
    let response = native_responses::events(
        State(restarted),
        Path(response_id),
        axum::extract::Query(native_responses::Cursor {
            starting_after: None,
        }),
        bearer(&key),
    )
    .await;
    let wire = body_text(response).await;
    assert_eq!(
        frames(&wire).last().unwrap()["response"]["output"][0]["content"][0]["text"],
        "Here is your answer"
    );
    let finished = engine::get(&state.db, OWNER, &id).await.unwrap();
    assert!(finished.pending_events.is_empty() && finished.active_turn.is_none());
    assert!(
        turns.insert(
            events
                .find_one(doc! {"_id":&stored.event_key})
                .await
                .unwrap()
                .unwrap()
                .turn_id
                .unwrap()
        )
    );
    assert_eq!(calls.lock().await.len(), 3);
    server.abort();
    state.db.drop().await.unwrap();
}
