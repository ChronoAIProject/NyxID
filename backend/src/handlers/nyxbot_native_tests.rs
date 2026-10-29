use super::*;
use crate::{
    models::{
        nyxbot_channel::GatewaySource,
        nyxbot_gateway_response::{COLLECTION_NAME as RESPONSES, GatewayResponse as Stored},
    },
    services::nyxbot_response_recovery as recovery,
};

async fn native_channel(state: &AppState) -> (NyxbotChannel, String) {
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
    let request = native_event(&row, "500", "Hello");
    let response = respond(&state, &key, &request, "native-500").await;
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
    saved.payload_ciphertext = recovery::Payload::new(
        &id,
        "nyxbot_stable",
        recovery::metadata(&request["metadata"]).unwrap(),
    )
    .seal(&state.encryption_keys)
    .await
    .unwrap();
    state
        .db
        .collection::<Stored>(RESPONSES)
        .replace_one(doc! {"_id":&saved.id}, &saved)
        .await
        .unwrap();
    state.db.collection::<NyxbotEvent>(EVENTS).update_one(doc! {"_id":&saved.event_key},doc! {"$set":{"created_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::minutes(3)),"status":"running"}}).await.unwrap();
    recovery::refresh(&state.db, &state.encryption_keys, &saved)
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
    let (state, calls, server) = setup("native_guild_identity").await;
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
        assert!(wire.contains("Here is your answer"), "{id}: {wire}");
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
    assert_eq!(
        native_responses::snapshot(
            State(state.clone()),
            Path(saved[0].response_id.clone()),
            bearer(&key)
        )
        .await
        .status(),
        StatusCode::OK
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
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{path, path_regex},
    };
    let (state, calls, upstream) = setup("native_discord_follow").await;
    let (row, key) = native_channel(&state).await;
    state.db.collection::<bson::Document>("feature_flag_overrides").insert_one(doc! {"_id":Uuid::new_v4().to_string(),"org_user_id":bson::Bson::Null,
        "flag_key":feature_flag_service::NYXBOT_THREAD_FOLLOW_FLAG_KEY,"target_kind":"user","target_key":OWNER,"enabled":true,
        "updated_by":"test","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
    let http = MockServer::start().await;
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
    Mock::given(path_regex("^/channels/201/messages/[0-9]+$")).respond_with(|r:&wiremock::Request| {
        let id=r.url.path().rsplit('/').next().unwrap();
        let mention=match id {"601"=>json!([{"id":"100"}]),"603"=>json!([{"id":"8"}]),_=>json!([])};
        ResponseTemplate::new(200).set_body_json(json!({"id":id,"channel_id":"201","type":0,
            "author":{"id":"7","bot":false},"content":if id=="604" {"stop following"} else {"Hello"},
            "mentions":mention,"mention_roles":[],"mention_everyone":false,"attachments":[]}))
    }).mount(&http).await;
    for (id, expect_turn) in [
        ("601", true),
        ("602", true),
        ("603", false),
        ("606", false),
        ("604", false),
        ("605", false),
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
    assert_eq!(child.follow.follow_state.as_deref(), Some("stopped"));
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

async fn native_flag(state: &AppState, enabled: bool) {
    state.db.collection::<bson::Document>("feature_flag_overrides").insert_one(doc! {
        "_id":Uuid::new_v4().to_string(),"org_user_id":bson::Bson::Null,
        "flag_key":"nyxbot:gateway-discord","target_kind":"user","target_key":OWNER,"enabled":enabled,
        "updated_by":"test","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
    }).await.unwrap();
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
        for (_, _, body) in writes.iter().filter(|c| c.0 == "POST" || c.0 == "PUT") {
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
    recovery::refresh(&state.db, &state.encryption_keys, &stored)
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
    let chat = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"channel_id":&row.id,"partition":&origin.partition})
        .await
        .unwrap()
        .unwrap();
    let plan=images::ImageSendPlan {attachments:vec![serde_json::from_value(json!({"id":"tool-image","content_type":"image/png","size":10,"label":"image","origin":"tool","chat_delivery":true})).unwrap()]};
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
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    for native_thread in [false, true] {
        let (state, _, upstream) = setup("native_async_thread_receipts").await;
        let (row, key) = native_channel(&state).await;
        state.db.collection::<bson::Document>("feature_flag_overrides").insert_one(doc! {
            "_id":Uuid::new_v4().to_string(),"org_user_id":bson::Bson::Null,
            "flag_key":feature_flag_service::NYXBOT_THREAD_FOLLOW_FLAG_KEY,
            "target_kind":"user","target_key":OWNER,"enabled":true,
            "updated_by":"test","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
        }).await.unwrap();
        let discord = MockServer::start().await;
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
        request["event_context"]["activity"]["conversation"] = json!({"kind":"group","id":"201","thread_id":if native_thread {Some("201")} else {None}});
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
        let gateway = MockServer::start().await;
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
            let result = native_responses::send_update(
                &state,
                &row,
                origin,
                stored.conversation_id.as_deref().unwrap(),
                "Update",
            )
            .await;
            assert_eq!(result.is_ok(), expected == "reply_confirmed", "{expected}");
            let requests = gateway.received_requests().await.unwrap();
            assert_eq!(
                requests.len(),
                1,
                "no automatic retry of an uncertain receipt"
            );
            let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
            assert_eq!(
                body.get("thread_reply"),
                native_thread.then_some(&Value::Bool(true))
            );
            let saved = load_channel(&state, OWNER, &row.id).await.unwrap();
            assert_eq!(saved.gateway_native_delivery.as_deref(), Some(expected));
        }
        upstream.abort();
    }
}
